use crate::agent::{self, ChatEntry, Event};
use crate::chrome;
use crate::diff::{self, FileTouch, Op};
use crate::glyph;
use crate::llm::{self, Abort};
use crate::mind::{self, SessionRec};
use crate::paths;
use crate::runes::{self, RuneKey};
use crossterm::{
    event::{self, Event as CEvent, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    layout::{Constraint, Direction, Layout, Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame, Terminal,
};
use serde_json::{json, Value};
use std::io::{self, stdout};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const PINK: Color = Color::Rgb(0xe8, 0x79, 0xa0);
const BLUSH: Color = Color::Rgb(0xf0, 0xb4, 0xc8);
const DIM: Color = Color::Rgb(0x9d, 0x5c, 0x75);
const FG: Color = Color::Rgb(0xff, 0xeb, 0xf2);
const SILVER: Color = Color::Rgb(0xc0, 0xc0, 0xc8);
const GREEN: Color = Color::Rgb(0x7d, 0xc9, 0x8a);
const RED: Color = Color::Rgb(0xe8, 0x7a, 0x7a);
const SPLASH_MIN: Duration = Duration::from_millis(1500);
const CARD_LINES: usize = 24;

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
    Tool {
        title: String,
        body: String,
        file: Option<FileTouch>,
    },
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

struct LookState {
    path: String,
    lines: Vec<(Op, String)>,
    scroll: u16,
}

struct App {
    chat: Vec<ChatEntry>,
    buf: String,
    cursor: usize,
    rune_idx: usize,
    session_id: String,
    verbose: bool,
    help: bool,
    busy: bool,
    waking: bool,
    splash: bool,
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
    born: Instant,
    look: Option<LookState>,
    file_sel: usize,
    file_hits: Vec<(Rect, usize)>,
    pending_editor: Option<PathBuf>,
}

impl App {
    fn rune_key(&self) -> RuneKey {
        runes::all()[self.rune_idx].key
    }
}

enum KeyOut {
    Continue,
    Quit,
}

fn load_verbose() -> bool {
    fs_bool(&paths::ui_path())
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
        (Vec::new(), 0, chrono_id())
    };
    let waking = !llm::llm_up();
    let mut app = App {
        chat,
        buf: String::new(),
        cursor: 0,
        rune_idx,
        session_id,
        verbose: verbose || load_verbose(),
        help: false,
        busy: false,
        waking,
        splash: true,
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
        born: Instant::now(),
        look: None,
        file_sel: 0,
        file_hits: Vec::new(),
        pending_editor: None,
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
    let _ = term.hide_cursor();
    let mut last_tick = Instant::now();
    loop {
        while let Ok(msg) = app.rx.try_recv() {
            handle_msg(&mut app, msg);
        }
        if app.splash && !app.waking && app.born.elapsed() >= SPLASH_MIN {
            app.splash = false;
        }
        term.draw(|f| draw(f, &mut app))?;
        if let Some(path) = app.pending_editor.take() {
            suspend_editor(&mut term, &path)?;
        }
        let timeout = Duration::from_millis(50).saturating_sub(last_tick.elapsed());
        if event::poll(timeout)? {
            match event::read()? {
                CEvent::Key(k) => match handle_key(&mut app, k)? {
                    KeyOut::Quit => break,
                    KeyOut::Continue => {}
                },
                CEvent::Mouse(m) => match m.kind {
                    MouseEventKind::ScrollUp => {
                        if let Some(look) = app.look.as_mut() {
                            look.scroll = look.scroll.saturating_add(1);
                        } else {
                            app.scroll = app.scroll.saturating_add(1);
                        }
                    }
                    MouseEventKind::ScrollDown => {
                        if let Some(look) = app.look.as_mut() {
                            look.scroll = look.scroll.saturating_sub(1);
                        } else {
                            app.scroll = app.scroll.saturating_sub(1);
                        }
                    }
                    MouseEventKind::Down(MouseButton::Left) => {
                        let col = m.column;
                        let row = m.row;
                        for (rect, fi) in &app.file_hits {
                            if col >= rect.x
                                && col < rect.x.saturating_add(rect.width)
                                && row >= rect.y
                                && row < rect.y.saturating_add(rect.height)
                            {
                                app.file_sel = *fi;
                            }
                        }
                    }
                    _ => {}
                },
                CEvent::Paste(s) => {
                    insert_text(&mut app.buf, &mut app.cursor, &s);
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
            app.phase = format!("round {n}/{cap}");
        }
        UiMsg::Tool { title, body, file } => {
            if file.is_some() {
                let n = file_card_count(app) + 1;
                app.file_sel = n.saturating_sub(1);
                app.scroll = 0;
            }
            app.chat.push(ChatEntry::Tool { title, body, file });
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
            app.scroll = 0;
            if wrote_plan {
                app.plan_menu = true;
                app.plan_idx = 0;
            }
        }
    }
}

fn handle_key(app: &mut App, k: KeyEvent) -> anyhow::Result<KeyOut> {
    if app.confirm.peek().is_some() {
        match k.code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => app.confirm.respond(true),
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => app.confirm.respond(false),
            _ => {}
        }
        return Ok(KeyOut::Continue);
    }
    if app.look.is_some() {
        match k.code {
            KeyCode::Esc | KeyCode::Char('q') if app.buf.is_empty() => app.look = None,
            KeyCode::Char('o') if app.buf.is_empty() => request_open(app),
            KeyCode::Char('[') if app.buf.is_empty() => {
                cycle_file(app, -1);
                open_look(app);
            }
            KeyCode::Char(']') if app.buf.is_empty() => {
                cycle_file(app, 1);
                open_look(app);
            }
            KeyCode::Up => {
                if let Some(l) = app.look.as_mut() {
                    l.scroll = l.scroll.saturating_add(1);
                }
            }
            KeyCode::Down => {
                if let Some(l) = app.look.as_mut() {
                    l.scroll = l.scroll.saturating_sub(1);
                }
            }
            KeyCode::PageUp => {
                if let Some(l) = app.look.as_mut() {
                    l.scroll = l.scroll.saturating_add(8);
                }
            }
            KeyCode::PageDown => {
                if let Some(l) = app.look.as_mut() {
                    l.scroll = l.scroll.saturating_sub(8);
                }
            }
            KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                if app.busy {
                    app.abort.cancel();
                }
                return Ok(KeyOut::Quit);
            }
            _ => {}
        }
        return Ok(KeyOut::Continue);
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
        return Ok(KeyOut::Continue);
    }

    match k.code {
        KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => {
            if app.busy {
                app.abort.cancel();
            }
            return Ok(KeyOut::Quit);
        }
        KeyCode::Char('q') if app.buf.is_empty() => {
            if app.busy {
                app.abort.cancel();
            }
            return Ok(KeyOut::Quit);
        }
        KeyCode::Esc => {
            if app.help {
                app.help = false;
            } else if app.buf.is_empty() {
                if app.busy {
                    app.abort.cancel();
                } else {
                    return Ok(KeyOut::Quit);
                }
            } else {
                app.buf.clear();
                app.cursor = 0;
            }
        }
        KeyCode::Char('v') if app.buf.is_empty() && !app.busy => {
            app.verbose = !app.verbose;
            save_verbose(app.verbose);
        }
        KeyCode::Char('?') if app.buf.is_empty() => app.help = !app.help,
        KeyCode::Tab => {
            app.rune_idx = runes::next_idx(app.rune_idx);
        }
        KeyCode::BackTab => {
            app.rune_idx = runes::prev_idx(app.rune_idx);
        }
        KeyCode::Char('r') if k.modifiers.contains(KeyModifiers::CONTROL) => {
            app.rune_idx = runes::next_idx(app.rune_idx);
        }
        KeyCode::Char('[') if app.buf.is_empty() => cycle_file(app, -1),
        KeyCode::Char(']') if app.buf.is_empty() => cycle_file(app, 1),
        KeyCode::Char('l') if app.buf.is_empty() => open_look(app),
        KeyCode::Char('o') if app.buf.is_empty() => request_open(app),
        KeyCode::Enter => {
            let text = app.buf.trim().to_string();
            if !text.is_empty() {
                app.buf.clear();
                app.cursor = 0;
                send(app, &text);
            }
        }
        KeyCode::Backspace => {
            delete_before(&mut app.buf, &mut app.cursor);
        }
        KeyCode::Delete => {
            delete_at(&mut app.buf, &mut app.cursor);
        }
        KeyCode::Left => {
            if app.cursor > 0 {
                app.cursor -= 1;
            }
        }
        KeyCode::Right => {
            let n = app.buf.chars().count();
            if app.cursor < n {
                app.cursor += 1;
            }
        }
        KeyCode::Home => app.cursor = 0,
        KeyCode::End => app.cursor = app.buf.chars().count(),
        KeyCode::Char(c) => {
            if k.modifiers.contains(KeyModifiers::CONTROL) {
                if c == 'u' {
                    app.buf.clear();
                    app.cursor = 0;
                }
            } else {
                insert_text(&mut app.buf, &mut app.cursor, &c.to_string());
            }
        }
        KeyCode::Up => app.scroll = app.scroll.saturating_add(1),
        KeyCode::Down => app.scroll = app.scroll.saturating_sub(1),
        KeyCode::PageUp => app.scroll = app.scroll.saturating_add(5),
        KeyCode::PageDown => app.scroll = app.scroll.saturating_sub(5),
        _ => {}
    }
    Ok(KeyOut::Continue)
}

fn send(app: &mut App, text: &str) {
    if app.waking {
        app.buf = text.to_string();
        app.cursor = app.buf.chars().count();
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
    app.scroll = 0;
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
                Event::ToolEnd {
                    title, body, file, ..
                } => {
                    let _ = tx.send(UiMsg::Tool { title, body, file });
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

fn file_card_count(app: &App) -> usize {
    app.chat
        .iter()
        .filter(|e| matches!(e, ChatEntry::Tool { file: Some(_), .. }))
        .count()
}

fn file_cards(app: &App) -> Vec<&FileTouch> {
    app.chat
        .iter()
        .filter_map(|e| match e {
            ChatEntry::Tool { file: Some(f), .. } => Some(f),
            _ => None,
        })
        .collect()
}

fn current_touch(app: &App) -> Option<&FileTouch> {
    let n = file_card_count(app);
    if n == 0 {
        return None;
    }
    let want = app.file_sel.min(n - 1);
    file_cards(app).into_iter().nth(want)
}

fn cycle_file(app: &mut App, delta: i32) {
    let n = file_card_count(app);
    if n == 0 {
        app.status = "pixie: no file to look at yet".into();
        return;
    }
    let cur = app.file_sel.min(n - 1) as i32;
    app.file_sel = ((cur + delta).rem_euclid(n as i32)) as usize;
}

fn open_look(app: &mut App) {
    let Some(touch) = current_touch(app) else {
        app.status = "pixie: no file to look at yet".into();
        return;
    };
    let path = touch.path.clone();
    let lines = look_lines(touch);
    app.look = Some(LookState {
        path,
        lines,
        scroll: 0,
    });
}

fn look_lines(touch: &FileTouch) -> Vec<(Op, String)> {
    if touch.binary {
        return vec![(Op::Eq, "not a text file".into())];
    }
    if !touch.lines.is_empty() {
        return touch
            .lines
            .iter()
            .map(|l| (l.op, l.text.clone()))
            .collect();
    }
    match std::fs::read_to_string(&touch.path) {
        Ok(t) => t.lines().map(|x| (Op::Eq, x.to_string())).collect(),
        Err(e) => vec![(Op::Eq, format!("pixie: could not read ({e})"))],
    }
}

fn request_open(app: &mut App) {
    let Some(touch) = current_touch(app) else {
        app.status = "pixie: no file to open yet".into();
        return;
    };
    match paths::assert_allowed(&touch.path) {
        Err(e) => app.status = format!("pixie: {e}"),
        Ok(p) => app.pending_editor = Some(p),
    }
}

fn suspend_editor<B: ratatui::backend::Backend>(
    term: &mut Terminal<B>,
    path: &Path,
) -> anyhow::Result<()> {
    disable_raw_mode()?;
    execute!(
        io::stdout(),
        LeaveAlternateScreen,
        event::DisableMouseCapture,
        event::DisableBracketedPaste
    )?;
    let _ = term.show_cursor();
    let argv = editor_argv();
    let mut cmd = std::process::Command::new(&argv[0]);
    cmd.args(&argv[1..]).arg(path);
    let _ = cmd.status();
    enable_raw_mode()?;
    execute!(
        io::stdout(),
        EnterAlternateScreen,
        event::EnableMouseCapture,
        event::EnableBracketedPaste
    )?;
    term.clear()?;
    Ok(())
}

fn editor_argv() -> Vec<String> {
    let raw = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "nano".into());
    mind::split_argv(&raw).unwrap_or_else(|_| vec![raw])
}

fn pretty_path(p: &str) -> String {
    let h = paths::home();
    let hs = h.to_string_lossy();
    if let Some(rest) = p.strip_prefix(hs.as_ref()) {
        format!("~{rest}")
    } else {
        p.to_string()
    }
}

fn prompt_text(app: &App, rune_desc: &str) -> (String, bool) {
    let spin = if app.busy || app.splash || app.waking {
        glyph::spin_line(app.born)
    } else {
        glyph::SPIN[0]
    };
    let wait = glyph::wait_line(app.born);
    if let Some((name, args)) = app.confirm.peek() {
        return (format!("{spin}  approve {name}? {args}  [y/N]"), false);
    }
    if !app.buf.is_empty() {
        return (format!("✦ {}", app.buf), true);
    }
    if app.splash || app.waking {
        return (format!("{spin}  {wait}"), false);
    }
    if app.busy {
        let phase = if app.phase.is_empty() {
            wait.to_string()
        } else {
            format!("{} · {wait}", app.phase)
        };
        return (
            format!("{spin}  {phase}  (type + enter barge-in)"),
            false,
        );
    }
    let hint = if app.status.is_empty() {
        format!("{rune_desc} · tab rune · ?")
    } else {
        app.status.clone()
    };
    (format!("{spin}  {hint}"), false)
}

fn byte_at(s: &str, char_i: usize) -> usize {
    s.char_indices()
        .nth(char_i)
        .map(|(i, _)| i)
        .unwrap_or(s.len())
}

fn insert_text(buf: &mut String, cursor: &mut usize, text: &str) {
    let i = byte_at(buf, *cursor);
    buf.insert_str(i, text);
    *cursor = cursor.saturating_add(text.chars().count());
}

fn delete_before(buf: &mut String, cursor: &mut usize) {
    if *cursor == 0 {
        return;
    }
    let end = byte_at(buf, *cursor);
    let start = byte_at(buf, *cursor - 1);
    buf.replace_range(start..end, "");
    *cursor -= 1;
}

fn delete_at(buf: &mut String, cursor: &mut usize) {
    let n = buf.chars().count();
    if *cursor >= n {
        return;
    }
    let start = byte_at(buf, *cursor);
    let end = byte_at(buf, *cursor + 1);
    buf.replace_range(start..end, "");
}

struct Bubble {
    right: bool,
    title: String,
    lines: Vec<Line<'static>>,
    max_frac: u16,
    border: Color,
    file_i: Option<usize>,
}

fn draw(f: &mut Frame, app: &mut App) {
    let size = f.area();
    let runes = runes::all();
    let rune = &runes[app.rune_idx];
    let (prompt_inner, editing) = prompt_text(app, rune.desc);
    let extra = if app.help || app.plan_menu { 6 } else { 0 };
    let prompt_h = chrome::prompt_height(&prompt_inner, size.width, size.height);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(5),
            Constraint::Length(extra),
            Constraint::Length(prompt_h),
            Constraint::Length(1),
        ])
        .split(size);

    app.file_hits.clear();

    if app.splash {
        draw_splash(f, chunks[0], app);
    } else if let Some(look) = app.look.as_ref() {
        draw_look(f, chunks[0], look);
    } else {
        draw_chat(f, chunks[0], app, rune.name);
    }

    if app.help {
        let help = Paragraph::new(
            "enter send · esc clear/cancel · q quit · tab / shift-tab runes · v verbose\n\
             l look file · o open in editor · [ ] files · Plan: y approve · e stay · n drop",
        )
        .style(Style::default().fg(SILVER))
        .block(
            Block::default()
                .title(" keys ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(DIM))
                .border_set(chrome::border_set()),
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
                    .border_style(Style::default().fg(PINK))
                    .border_set(chrome::border_set()),
            ),
            chunks[1],
        );
    }

    f.render_widget(
        Paragraph::new(prompt_inner.as_str())
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(BLUSH))
                    .border_set(chrome::border_set()),
            ),
        chunks[2],
    );
    if editing {
        let inner_w = chrome::prompt_inner_width(chunks[2].width);
        let prefix = "✦ ".chars().count();
        let vis = prefix + app.cursor.min(app.buf.chars().count());
        let (row, col) = chrome::wrap_cursor(&prompt_inner, inner_w, vis);
        let max_row = chunks[2].height.saturating_sub(2);
        let row = row.min(max_row.saturating_sub(1));
        let x = chunks[2].x.saturating_add(1).saturating_add(col);
        let y = chunks[2].y.saturating_add(1).saturating_add(row);
        if x < chunks[2].x.saturating_add(chunks[2].width)
            && y < chunks[2].y.saturating_add(chunks[2].height)
        {
            f.set_cursor_position(Position { x, y });
        }
    }

    let pills: Vec<Span> = runes
        .iter()
        .enumerate()
        .flat_map(|(i, r)| {
            let gap = if i == 0 {
                None
            } else {
                Some(Span::raw("  "))
            };
            let style = if i == app.rune_idx {
                Style::default().fg(PINK).add_modifier(Modifier::REVERSED)
            } else {
                Style::default().fg(SILVER)
            };
            let pill = Span::styled(format!(" {} ", r.name), style);
            match gap {
                Some(g) => vec![g, pill],
                None => vec![pill],
            }
        })
        .collect();
    let mut footer = pills;
    footer.push(Span::styled(
        if app.verbose { "  · ?  verbose" } else { "  · ?" },
        Style::default().fg(SILVER),
    ));
    f.render_widget(Paragraph::new(Line::from(footer)), chunks[3]);
}

fn draw_splash(f: &mut Frame, area: Rect, app: &App) {
    f.render_widget(Clear, area);
    let frame = glyph::splash_frame(app.born);
    let gh = frame.len() as u16;
    let gw = 7u16;
    let x = area.x + area.width.saturating_sub(gw) / 2;
    let y = area.y + area.height.saturating_sub(gh + 2) / 2;
    for (i, line) in frame.iter().enumerate() {
        let row = Rect {
            x,
            y: y.saturating_add(i as u16),
            width: gw,
            height: 1,
        };
        if row.y >= area.y + area.height {
            break;
        }
        f.render_widget(
            Paragraph::new(*line).style(Style::default().fg(PINK)),
            row,
        );
    }
    let wait = glyph::wait_line(app.born);
    let wy = y.saturating_add(gh).saturating_add(1);
    if wy < area.y + area.height {
        let w = (wait.chars().count() as u16).min(area.width);
        let wx = area.x + area.width.saturating_sub(w) / 2;
        f.render_widget(
            Paragraph::new(wait).style(Style::default().fg(BLUSH)),
            Rect {
                x: wx,
                y: wy,
                width: w.max(1),
                height: 1,
            },
        );
    }
}

fn draw_look(f: &mut Frame, area: Rect, look: &LookState) {
    let title = format!(" look · {} ", pretty_path(&look.path));
    let inner_w = area.width.saturating_sub(2).max(8) as usize;
    let mut lines: Vec<Line> = Vec::new();
    for (op, text) in &look.lines {
        for chunk in chrome::wrap(text, inner_w.saturating_sub(2)) {
            lines.push(diff_line(*op, &chunk));
        }
    }
    if lines.is_empty() {
        lines.push(Line::from(Span::styled("(empty)", Style::default().fg(DIM))));
    }
    let para = Paragraph::new(lines)
        .block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(PINK))
                .border_set(chrome::border_set()),
        )
        .wrap(Wrap { trim: false })
        .scroll((look.scroll, 0));
    f.render_widget(para, area);
}

fn draw_chat(f: &mut Frame, area: Rect, app: &mut App, rune_name: &str) {
    let title = format!(" ✦ Pixie ✦ {rune_name} · {} ✦ ", app.session_id);
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(PINK))
        .border_set(chrome::border_set())
        .style(Style::default().fg(FG));
    f.render_widget(block, area);
    let inner = Rect {
        x: area.x.saturating_add(1),
        y: area.y.saturating_add(1),
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    };
    if inner.width < 8 || inner.height < 2 {
        return;
    }

    let bubbles = collect_bubbles(app, inner.width);
    let mut heights = Vec::new();
    for b in &bubbles {
        let max_w = ((inner.width as u32 * b.max_frac as u32) / 100).clamp(12, inner.width as u32) as u16;
        let body = bubble_body_text(b);
        let w = chrome::bubble_width(&body, max_w, 12).min(inner.width);
        let inner_w = w.saturating_sub(2).max(1) as usize;
        let h = 2u16 + wrapped_height(&b.lines, inner_w);
        heights.push((w, h.max(3)));
    }

    let gaps = bubbles.len().saturating_sub(1) as u16;
    let total: u16 = heights.iter().map(|(_, h)| *h).sum::<u16>().saturating_add(gaps);
    let view = inner.height;
    let max_scroll = total.saturating_sub(view);
    if app.scroll > max_scroll {
        app.scroll = max_scroll;
    }
    let mut cursor = if total <= view {
        inner.y as i32 + (view - total) as i32
    } else {
        inner.y as i32 - max_scroll.saturating_sub(app.scroll) as i32
    };
    let bottom = (inner.y + inner.height) as i32;
    for (i, b) in bubbles.iter().enumerate() {
        let (w, h) = heights[i];
        let x = if b.right {
            inner.x + inner.width.saturating_sub(w)
        } else {
            inner.x
        };
        let y = cursor;
        cursor += h as i32 + 1;
        if y + h as i32 <= inner.y as i32 {
            continue;
        }
        if y >= bottom {
            break;
        }
        if y < inner.y as i32 {
            continue;
        }
        let h_vis = (bottom - y).min(h as i32) as u16;
        if h_vis < 3 {
            continue;
        }
        let r = Rect {
            x,
            y: y as u16,
            width: w,
            height: h_vis,
        };
        render_bubble(f, r, b, app.file_sel);
        if let Some(fi) = b.file_i {
            app.file_hits.push((r, fi));
        }
    }
}

fn collect_bubbles(app: &App, _width: u16) -> Vec<Bubble> {
    let mut out = Vec::new();
    let mut file_i = 0usize;
    for e in &app.chat {
        match e {
            ChatEntry::User(t) => {
                out.push(Bubble {
                    right: true,
                    title: " you ".into(),
                    lines: wrap_plain(t),
                    max_frac: 70,
                    border: BLUSH,
                    file_i: None,
                });
            }
            ChatEntry::Assistant {
                text,
                clock,
                think,
                cancelled,
            } => {
                if app.verbose && !think.is_empty() {
                    out.push(Bubble {
                        right: false,
                        title: " think ".into(),
                        lines: wrap_plain(think),
                        max_frac: 80,
                        border: DIM,
                        file_i: None,
                    });
                }
                let mark = if *cancelled { " (cancelled)" } else { "" };
                out.push(Bubble {
                    right: false,
                    title: format!(" ✦ Pixie · {clock}{mark} "),
                    lines: wrap_plain(text),
                    max_frac: 80,
                    border: PINK,
                    file_i: None,
                });
            }
            ChatEntry::Tool { title, body, file } => {
                if let Some(ft) = file {
                    let sel = file_i == app.file_sel;
                    out.push(file_bubble(ft, sel, app.verbose, body, file_i));
                    file_i += 1;
                } else if app.verbose {
                    out.push(Bubble {
                        right: false,
                        title: format!(" {title} "),
                        lines: wrap_plain(body),
                        max_frac: 90,
                        border: DIM,
                        file_i: None,
                    });
                } else {
                    out.push(Bubble {
                        right: false,
                        title: " tool ".into(),
                        lines: wrap_plain(title),
                        max_frac: 90,
                        border: DIM,
                        file_i: None,
                    });
                }
            }
        }
    }
    if app.busy {
        if app.verbose && !app.live_think.is_empty() {
            out.push(Bubble {
                right: false,
                title: " think ".into(),
                lines: wrap_plain(&app.live_think),
                max_frac: 80,
                border: DIM,
                file_i: None,
            });
        }
        let vis = crate::think::strip(&app.live);
        let spin = glyph::spin_line(app.born);
        if !vis.is_empty() {
            out.push(Bubble {
                right: false,
                title: format!(" ✦ Pixie · {spin} "),
                lines: wrap_plain(&vis),
                max_frac: 80,
                border: PINK,
                file_i: None,
            });
        } else {
            out.push(Bubble {
                right: false,
                title: format!(" ✦ Pixie · {spin} "),
                lines: wrap_plain(glyph::wait_line(app.born)),
                max_frac: 50,
                border: SILVER,
                file_i: None,
            });
        }
    }
    out
}

fn file_bubble(ft: &FileTouch, sel: bool, verbose: bool, body: &str, file_i: usize) -> Bubble {
    let sum = ft.summary();
    let mark = if sel { "▸ " } else { "" };
    let title = if sum.is_empty() {
        format!(" {mark}{} · {} ", ft.kind.as_str(), pretty_path(&ft.path))
    } else {
        format!(
            " {mark}{} · {} · {sum} ",
            ft.kind.as_str(),
            pretty_path(&ft.path)
        )
    };
    let mut lines: Vec<Line<'static>> = Vec::new();
    if ft.kind == diff::FileKind::Search && !ft.extra_paths.is_empty() {
        for p in ft.extra_paths.iter().take(12) {
            lines.push(Line::from(Span::styled(
                pretty_path(p),
                Style::default().fg(SILVER),
            )));
        }
    } else if ft.binary {
        lines.push(Line::from(Span::styled(
            "not a text file",
            Style::default().fg(DIM),
        )));
    } else {
        for l in diff::for_card(&ft.lines, CARD_LINES) {
            lines.push(diff_line(l.op, &l.text));
        }
    }
    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "(empty)",
            Style::default().fg(DIM),
        )));
    }
    lines.push(Line::from(Span::styled(
        "o open  ·  l look  ·  [ ] files",
        Style::default().fg(DIM),
    )));
    if verbose && !body.is_empty() {
        for chunk in chrome::wrap(body, 80) {
            lines.push(Line::from(Span::styled(
                chunk,
                Style::default().fg(DIM).add_modifier(Modifier::ITALIC),
            )));
        }
    }
    Bubble {
        right: false,
        title,
        lines,
        max_frac: 90,
        border: if sel { PINK } else { DIM },
        file_i: Some(file_i),
    }
}

fn diff_line(op: Op, text: &str) -> Line<'static> {
    match op {
        Op::Add => Line::from(Span::styled(
            format!("+ {text}"),
            Style::default().fg(GREEN),
        )),
        Op::Del => Line::from(Span::styled(
            format!("- {text}"),
            Style::default().fg(RED),
        )),
        Op::Eq => Line::from(Span::styled(
            format!("  {text}"),
            Style::default().fg(SILVER),
        )),
    }
}

fn wrap_plain(text: &str) -> Vec<Line<'static>> {
    if text.is_empty() {
        return vec![Line::from("")];
    }
    text.lines()
        .map(|l| Line::from(l.to_string()))
        .collect()
}

fn bubble_body_text(b: &Bubble) -> String {
    let mut s = b.title.clone();
    for line in &b.lines {
        s.push('\n');
        s.push_str(&line.to_string());
    }
    s
}

fn wrapped_height(lines: &[Line<'static>], inner_w: usize) -> u16 {
    let mut n = 0u16;
    for line in lines {
        let t = line.to_string();
        let chunks = chrome::wrap(&t, inner_w.max(1));
        n = n.saturating_add(chunks.len().max(1) as u16);
    }
    n.max(1)
}

fn render_bubble(f: &mut Frame, rect: Rect, b: &Bubble, file_sel: usize) {
    let border = if b.file_i == Some(file_sel) && b.file_i.is_some() {
        PINK
    } else {
        b.border
    };
    let para = Paragraph::new(b.lines.clone())
        .block(
            Block::default()
                .title(b.title.clone())
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border))
                .border_set(chrome::border_set()),
        )
        .wrap(Wrap { trim: false });
    f.render_widget(para, rect);
}
