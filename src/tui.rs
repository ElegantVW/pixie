use crate::agent::{self, ChatEntry, Event};
use crate::llm::{self, Abort};
use crate::mind::{self, SessionRec};
use crate::paths;
use crate::runes::{self, RuneKey};
use crossterm::{
    event::{self, Event as CEvent, KeyCode, KeyEvent, KeyModifiers, MouseEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame, Terminal,
};
use serde_json::{json, Value};
use std::io::{self, stdout};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const PINK: Color = Color::Rgb(0xe8, 0x79, 0xa0);
const BLUSH: Color = Color::Rgb(0xf0, 0xb4, 0xc8);
const DIM: Color = Color::Rgb(0x9d, 0x5c, 0x75);
const FG: Color = Color::Rgb(0xff, 0xeb, 0xf2);
const SILVER: Color = Color::Rgb(0xc0, 0xc0, 0xc8);

struct ConfirmGate {
    pending: Mutex<Option<(String, Value)>>,
    answer: Mutex<Option<bool>>,
}

impl ConfirmGate {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            pending: Mutex::new(None),
            answer: Mutex::new(None),
        })
    }

    fn ask(&self, name: &str, args: &Value) -> bool {
        {
            let mut p = self.pending.lock().unwrap();
            *p = Some((name.to_string(), args.clone()));
            *self.answer.lock().unwrap() = None;
        }
        loop {
            if let Some(a) = *self.answer.lock().unwrap() {
                *self.pending.lock().unwrap() = None;
                return a;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn respond(&self, yes: bool) {
        *self.answer.lock().unwrap() = Some(yes);
    }

    fn peek(&self) -> Option<(String, Value)> {
        self.pending.lock().unwrap().clone()
    }
}

enum UiMsg {
    Chunk(String),
    Round { n: usize, cap: usize },
    Tool { title: String, body: String },
    Wake { err: Option<String> },
    Done {
        gen: u64,
        text: String,
        think: String,
        cancelled: bool,
        err: Option<String>,
        wrote_plan: bool,
    },
}

struct App {
    chat: Vec<ChatEntry>,
    buf: String,
    rune_idx: usize,
    session_id: String,
    verbose: bool,
    help: bool,
    busy: bool,
    waking: bool,
    live: String,
    live_think: String,
    phase: String,
    abort: Abort,
    confirm: Arc<ConfirmGate>,
    rx: Receiver<UiMsg>,
    tx: Sender<UiMsg>,
    scroll: u16,
    plan_menu: bool,
    plan_idx: usize,
    status: String,
    gen: u64,
}

impl App {
    fn rune_key(&self) -> RuneKey {
        runes::all()[self.rune_idx].key
    }
}

fn load_verbose() -> bool {
    let p = paths::ui_path();
    fs_bool(&p)
}

fn fs_bool(p: &std::path::Path) -> bool {
    std::fs::read_to_string(p)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.get("verbose").and_then(|x| x.as_bool()))
        .unwrap_or(false)
}

fn save_verbose(v: bool) {
    let p = paths::ui_path();
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(p, json!({"verbose": v}).to_string());
}

pub fn run(verbose: bool, resume: Option<String>) -> anyhow::Result<()> {
    let _ = mind::seed_memory(None);
    let (tx, rx) = mpsc::channel();
    let (chat, rune_idx, session_id) = if let Some(id) = resume {
        let (c, k) = agent::from_session(&id);
        let idx = runes::all().iter().position(|r| r.key == k).unwrap_or(0);
        (c, idx, id)
    } else {
        (
            Vec::new(),
            0,
            chrono_id(),
        )
    };
    let mut app = App {
        chat,
        buf: String::new(),
        rune_idx,
        session_id,
        verbose: verbose || load_verbose(),
        help: false,
        busy: false,
        waking: !llm::llm_up(),
        live: String::new(),
        live_think: String::new(),
        phase: String::new(),
        abort: Abort::new(),
        confirm: ConfirmGate::new(),
        rx,
        tx,
        scroll: 0,
        plan_menu: false,
        plan_idx: 0,
        status: String::new(),
        gen: 0,
    };
    if app.waking {
        let tx = app.tx.clone();
        std::thread::spawn(move || {
            let ok = llm::ensure_llm_up();
            let _ = tx.send(UiMsg::Wake {
                err: if ok {
                    None
                } else {
                    Some("menagerie did not wake".into())
                },
            });
        });
    }

    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        event::EnableMouseCapture,
        event::EnableBracketedPaste
    )?;
    let mut term = Terminal::new(ratatui::backend::CrosstermBackend::new(stdout))?;
    let mut last_tick = Instant::now();
    loop {
        while let Ok(msg) = app.rx.try_recv() {
            handle_msg(&mut app, msg);
        }
        term.draw(|f| draw(f, &app))?;
        let timeout = Duration::from_millis(50).saturating_sub(last_tick.elapsed());
        if event::poll(timeout)? {
            match event::read()? {
                CEvent::Key(k) => {
                    if handle_key(&mut app, k)? {
                        break;
                    }
                }
                CEvent::Mouse(m) => match m.kind {
                    MouseEventKind::ScrollUp => app.scroll = app.scroll.saturating_add(1),
                    MouseEventKind::ScrollDown => app.scroll = app.scroll.saturating_sub(1),
                    _ => {}
                },
                CEvent::Paste(s) => {
                    app.buf.push_str(&s);
                }
                CEvent::Resize(_, _) => {}
                _ => {}
            }
        }
        last_tick = Instant::now();
    }
    disable_raw_mode()?;
    execute!(
        io::stdout(),
        LeaveAlternateScreen,
        event::DisableMouseCapture,
        event::DisableBracketedPaste
    )?;
    if std::env::var("PIXIE_STOP_LLM_ON_EXIT").is_ok() {
        llm::unload();
    }
    Ok(())
}

fn chrono_id() -> String {
    std::process::Command::new("date")
        .args(["+%Y%m%d-%H%M%S"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("{}", std::process::id()))
}

fn handle_msg(app: &mut App, msg: UiMsg) {
    match msg {
        UiMsg::Chunk(s) => {
            app.live.push_str(&s);
            let sp = crate::think::split(&app.live);
            app.live_think = sp.think;
        }
        UiMsg::Round { n, cap } => {
            app.phase = format!("round {n}/{cap} · thinking…");
        }
        UiMsg::Tool { title, body } => {
            app.chat.push(ChatEntry::Tool { title, body });
            app.live.clear();
        }
        UiMsg::Wake { err } => {
            app.waking = false;
            if let Some(e) = err {
                app.status = format!("pixie: {e}");
            }
            if !app.buf.is_empty() && !app.busy {
                let t = std::mem::take(&mut app.buf);
                send(app, &t);
            }
        }
        UiMsg::Done {
            gen,
            text,
            think,
            cancelled,
            err,
            wrote_plan,
        } => {
            if gen != app.gen {
                return;
            }
            app.busy = false;
            if cancelled {
                if !text.trim().is_empty() {
                    app.chat.push(ChatEntry::Assistant {
                        text,
                        clock: agent::clock_now(),
                        think,
                        cancelled: true,
                    });
                }
            } else if let Some(e) = err {
                let msg = format!("(✦ pixie lost the connection: {e})");
                app.chat.push(ChatEntry::Assistant {
                    text: msg.clone(),
                    clock: agent::clock_now(),
                    think: String::new(),
                    cancelled: false,
                });
                mind::append_session(
                    &app.session_id,
                    &SessionRec {
                        ts: mind::now_ts(),
                        role: "assistant".into(),
                        rune: app.rune_key().as_str().into(),
                        text: msg,
                        tool: String::new(),
                        args: None,
                        result: None,
                        raw: String::new(),
                        think: String::new(),
                        cancelled: false,
                    },
                    None,
                );
            } else if !text.trim().is_empty() {
                app.chat.push(ChatEntry::Assistant {
                    text: text.clone(),
                    clock: agent::clock_now(),
                    think: think.clone(),
                    cancelled: false,
                });
                mind::append_session(
                    &app.session_id,
                    &SessionRec {
                        ts: mind::now_ts(),
                        role: "assistant".into(),
                        rune: app.rune_key().as_str().into(),
                        text,
                        tool: String::new(),
                        args: None,
                        result: None,
                        raw: String::new(),
                        think,
                        cancelled: false,
                    },
                    None,
                );
            }
            app.live.clear();
            app.phase.clear();
            if wrote_plan {
                app.plan_menu = true;
                app.plan_idx = 0;
            }
        }
    }
}

fn handle_key(app: &mut App, k: KeyEvent) -> anyhow::Result<bool> {
    if app.confirm.peek().is_some() {
        match k.code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => app.confirm.respond(true),
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => app.confirm.respond(false),
            _ => {}
        }
        return Ok(false);
    }
    if app.plan_menu && app.buf.is_empty() {
        match k.code {
            KeyCode::Esc => app.plan_menu = false,
            KeyCode::Char('j') | KeyCode::Down => {
                app.plan_idx = (app.plan_idx + 1) % 3;
            }
            KeyCode::Char('k') | KeyCode::Up => {
                app.plan_idx = (app.plan_idx + 2) % 3;
            }
            KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('e') | KeyCode::Char('n') => {
                let act = match k.code {
                    KeyCode::Char('y') => "approve",
                    KeyCode::Char('e') => "edit",
                    KeyCode::Char('n') => "drop",
                    _ => mind::plan_menu_choices()[app.plan_idx].2,
                };
                app.plan_menu = false;
                match act {
                    "approve" => {
                        app.rune_idx = runes::all()
                            .iter()
                            .position(|r| r.key == RuneKey::Build)
                            .unwrap_or(app.rune_idx);
                        send(app, "Implement the active plan.");
                    }
                    "drop" => {
                        mind::drop_plan(&app.session_id);
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        return Ok(false);
    }

    match k.code {
        KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => {
            if app.busy {
                app.abort.cancel();
            }
            return Ok(true);
        }
        KeyCode::Char('q') if app.buf.is_empty() => {
            if app.busy {
                app.abort.cancel();
            }
            return Ok(true);
        }
        KeyCode::Esc => {
            if app.help {
                app.help = false;
            } else if app.buf.is_empty() {
                if app.busy {
                    app.abort.cancel();
                } else {
                    return Ok(true);
                }
            } else {
                app.buf.clear();
            }
        }
        KeyCode::Char('v') if app.buf.is_empty() && !app.busy => {
            app.verbose = !app.verbose;
            save_verbose(app.verbose);
        }
        KeyCode::Char('?') if app.buf.is_empty() => app.help = !app.help,
        KeyCode::Tab | KeyCode::Char('r') if k.modifiers.contains(KeyModifiers::CONTROL) => {
            app.rune_idx = (app.rune_idx + 1) % 4;
        }
        KeyCode::Char(c) if app.buf.is_empty() && c.is_ascii_digit() && ('1'..='4').contains(&c) => {
            app.rune_idx = (c as u8 - b'1') as usize;
        }
        KeyCode::Enter => {
            let text = app.buf.trim().to_string();
            if !text.is_empty() {
                app.buf.clear();
                send(app, &text);
            }
        }
        KeyCode::Backspace => {
            app.buf.pop();
        }
        KeyCode::Char(c) => {
            if k.modifiers.contains(KeyModifiers::CONTROL) {
                if c == 'u' {
                    app.buf.clear();
                }
            } else {
                app.buf.push(c);
            }
        }
        KeyCode::Up => app.scroll = app.scroll.saturating_add(1),
        KeyCode::Down => app.scroll = app.scroll.saturating_sub(1),
        KeyCode::PageUp => app.scroll = app.scroll.saturating_add(5),
        KeyCode::PageDown => app.scroll = app.scroll.saturating_sub(5),
        _ => {}
    }
    Ok(false)
}

fn send(app: &mut App, text: &str) {
    if app.waking {
        app.buf = text.to_string();
        app.status = "waking the menagerie brain…".into();
        return;
    }
    if app.busy {
        app.abort.cancel();
        std::thread::sleep(Duration::from_millis(80));
    }
    app.gen += 1;
    let gen = app.gen;
    app.abort = Abort::new();
    app.chat.push(ChatEntry::User(text.to_string()));
    mind::append_session(
        &app.session_id,
        &SessionRec {
            ts: mind::now_ts(),
            role: "user".into(),
            rune: app.rune_key().as_str().into(),
            text: text.to_string(),
            tool: String::new(),
            args: None,
            result: None,
            raw: String::new(),
            think: String::new(),
            cancelled: false,
        },
        None,
    );
    app.busy = true;
    app.live.clear();
    app.live_think.clear();
    let rune = runes::all()[app.rune_idx].clone();
    let session = app.session_id.clone();
    let verbose = app.verbose;
    let abort = app.abort.clone();
    let confirm = app.confirm.clone();
    let tx = app.tx.clone();
    let mut owned = app.chat.clone();
    let rounds = rune.rounds;
    std::thread::spawn(move || {
        if !llm::llm_up() {
            llm::ensure_llm_up();
        }
        let gate = confirm.clone();
        let confirm_fn = move |name: &str, args: &Value| gate.ask(name, args);
        let out = agent::run_turn(
            &mut owned,
            &rune,
            &session,
            &abort,
            verbose,
            Some(&confirm_fn),
            |ev| match ev {
                Event::Chunk(s) => {
                    let _ = tx.send(UiMsg::Chunk(s));
                }
                Event::Round { n, cap } => {
                    let _ = tx.send(UiMsg::Round { n, cap });
                }
                Event::ToolEnd { title, body, .. } => {
                    let _ = tx.send(UiMsg::Tool { title, body });
                }
                Event::ToolStart { name, .. } => {
                    let _ = tx.send(UiMsg::Round { n: 0, cap: rounds });
                    let _ = tx.send(UiMsg::Chunk(format!("\n[{name}]")));
                }
            },
        );
        let _ = tx.send(UiMsg::Done {
            gen,
            text: out.text,
            think: out.think,
            cancelled: out.cancelled,
            err: out.err,
            wrote_plan: out.wrote_plan,
        });
    });
}

fn draw(f: &mut Frame, app: &App) {
    let size = f.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(5),
            Constraint::Length(if app.help || app.plan_menu { 6 } else { 0 }),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(size);

    let runes = runes::all();
    let rune = &runes[app.rune_idx];
    let title = format!(" ✦ Pixie ✦ {} · {} ✦ ", rune.name, app.session_id);
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(PINK))
        .style(Style::default().fg(FG));

    let mut lines: Vec<Line> = Vec::new();
    for e in &app.chat {
        match e {
            ChatEntry::User(t) => {
                lines.push(Line::from(Span::styled(
                    format!("you · {t}"),
                    Style::default().fg(BLUSH),
                )));
            }
            ChatEntry::Assistant {
                text,
                clock,
                think,
                cancelled,
            } => {
                if app.verbose && !think.is_empty() {
                    lines.push(Line::from(Span::styled(
                        format!("think · {think}"),
                        Style::default().fg(DIM).add_modifier(Modifier::ITALIC),
                    )));
                }
                let mark = if *cancelled { " (cancelled)" } else { "" };
                lines.push(Line::from(Span::styled(
                    format!("Pixie ✦ {clock}{mark} · {text}"),
                    Style::default().fg(PINK),
                )));
            }
            ChatEntry::Tool { title, body } => {
                let shown = if app.verbose {
                    format!("{title}\n{body}")
                } else {
                    title.clone()
                };
                lines.push(Line::from(Span::styled(shown, Style::default().fg(DIM))));
            }
        }
        lines.push(Line::from(""));
    }
    if app.busy {
        if app.verbose && !app.live_think.is_empty() {
            lines.push(Line::from(Span::styled(
                format!("think · {}", app.live_think),
                Style::default().fg(DIM).add_modifier(Modifier::ITALIC),
            )));
        }
        let vis = crate::think::strip(&app.live);
        if !vis.is_empty() {
            lines.push(Line::from(Span::styled(
                format!("Pixie · {vis}"),
                Style::default().fg(PINK),
            )));
        } else if !app.phase.is_empty() {
            lines.push(Line::from(Span::styled(
                format!("✦ {}", app.phase),
                Style::default().fg(SILVER),
            )));
        }
    }
    let para = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((app.scroll, 0));
    f.render_widget(para, chunks[0]);

    if app.help {
        let help = Paragraph::new(
            "enter send · esc clear/cancel · q quit · 1–4 runes · tab cycle · v verbose\n\
             Plan: after write_plan, y approve → Build · e stay · n drop",
        )
        .style(Style::default().fg(SILVER))
        .block(
            Block::default()
                .title(" keys ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(DIM)),
        );
        f.render_widget(help, chunks[1]);
    } else if app.plan_menu {
        let card = mind::plan_summary(&app.session_id, 4);
        let choices = mind::plan_menu_choices();
        let mut t = card;
        t.push('\n');
        for (i, (k, label, _)) in choices.iter().enumerate() {
            let mark = if i == app.plan_idx { "▸ " } else { "  " };
            t.push_str(&format!("{mark}{k}  {label}\n"));
        }
        f.render_widget(
            Paragraph::new(t).block(
                Block::default()
                    .title(" plan ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(PINK)),
            ),
            chunks[1],
        );
    }

    let prompt_inner = if let Some((name, args)) = app.confirm.peek() {
        format!("approve {name}? {args}  [y/N]")
    } else if app.waking {
        "✦ waking the menagerie brain…".into()
    } else if app.busy {
        format!("✦ {}  (type + enter barge-in)", if app.phase.is_empty() { "thinking…".into() } else { app.phase.clone() })
    } else if app.buf.is_empty() {
        format!("✦ {} · ↑↓ · ?", rune.desc)
    } else {
        format!("✦ {}", app.buf)
    };
    f.render_widget(
        Paragraph::new(prompt_inner).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(BLUSH)),
        ),
        chunks[2],
    );

    let pills: String = runes
        .iter()
        .enumerate()
        .map(|(i, r)| {
            if i == app.rune_idx {
                format!("[{} {}]", i + 1, r.name)
            } else {
                format!(" {} {} ", i + 1, r.name)
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    let verb = if app.verbose { " verbose" } else { "" };
    f.render_widget(
        Paragraph::new(format!("{pills}  · ?{verb}")).style(Style::default().fg(SILVER)),
        chunks[3],
    );
    let _ = chunks;
    let _unused: Option<Rect> = None;
    let _ = _unused;
}
