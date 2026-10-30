use crate::agent::{self, ChatEntry, Event};
use crate::attach::{self, Attach};
use crate::chrome;
use crate::diff::{self, FileTouch, Op};
use crate::glyph;
use crate::kitty;
use crate::llm::{self, Abort};
use crate::mind::{self, SessionRec};
use crate::paths;
use crate::prefs;
use crate::privilege;
use crate::runes::{self, RuneKey};
use crate::tools;
use crossterm::{
    event::{
        self, Event as CEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton,
        MouseEventKind,
    },
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
use std::collections::HashMap;
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
    Calling(String),
    Round {
        n: usize,
        cap: usize,
    },
    Tool {
        title: String,
        body: String,
        file: Option<FileTouch>,
        name: String,
        think: String,
        ok: bool,
    },
    Wake {
        err: Option<String>,
    },
    Titled {
        session: String,
        name: String,
    },
    Compacted {
        session: String,
        text: String,
        at: usize,
    },
    Blurb {
        id: String,
        text: String,
    },
    Mouth {
        err: Option<String>,
    },
    SudoReady {
        err: Option<String>,
    },
    Spoke {
        err: Option<String>,
    },
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
    image: bool,
}

struct App {
    chat: Vec<ChatEntry>,
    buf: String,
    cursor: usize,
    rune_idx: usize,
    session_id: String,
    session_name: String,
    verbose: bool,
    sparkle: bool,
    speak_on: bool,
    tongue: crate::speak::Tongue,
    default_rune: RuneKey,
    build_audit_rounds: bool,
    sudo_less_safe: bool,
    enhance_image: bool,
    art_dir: String,
    help: bool,
    busy: bool,
    waking: bool,
    splash: bool,
    door: bool,
    options: bool,
    opt_idx: usize,
    opt_hits: Vec<Rect>,
    mouth: prefs::Mouth,
    mouth_note: String,
    mouth_busy: bool,
    compacting: bool,
    sudo_buf: Option<String>,
    sudo_spin: u8,
    wake_err: Option<String>,
    wake_retry: Instant,
    mouth_role: String,
    delete_ask: bool,
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
    session_menu: bool,
    session_idx: usize,
    session_rows: Vec<mind::SessionRow>,
    rename: Option<String>,
    dash_hits: Vec<(Rect, DashHit)>,
    dash_click: Option<(Instant, usize)>,
    status: String,
    gen: u64,
    born: Instant,
    look: Option<LookState>,
    file_sel: usize,
    file_hits: Vec<(Rect, usize)>,
    pending_editor: Option<PathBuf>,
    pending_picker: bool,
    pending_files: Vec<Attach>,
    kitty_frame: HashMap<u32, kitty::Place>,
    kitty_sent: HashMap<u32, (u16, u16)>,
}

#[derive(Clone, Copy)]
enum DashHit {
    Session(usize),
    New,
    Options,
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

fn toggle_speak(app: &mut App) {
    app.speak_on = !app.speak_on;
    persist_ui(app);
    if app.speak_on {
        apply_tongue(app);
    } else {
        crate::speak::stop_sidecar();
        app.status = "tongue off".into();
    }
}

fn cycle_voice(app: &mut App, dir: i32) {
    let i = if !app.speak_on {
        0
    } else if app.tongue == crate::speak::Tongue::Vivian {
        1
    } else {
        2
    };
    match (i + dir).rem_euclid(3) {
        0 => {
            app.speak_on = false;
            persist_ui(app);
            crate::speak::stop_sidecar();
            app.status = "tongue off".into();
        }
        1 => {
            app.speak_on = true;
            app.tongue = crate::speak::Tongue::Vivian;
            persist_ui(app);
            apply_tongue(app);
        }
        _ => {
            app.speak_on = true;
            app.tongue = crate::speak::Tongue::Design;
            persist_ui(app);
            apply_tongue(app);
        }
    }
}

fn apply_tongue(app: &mut App) {
    if !crate::speak::ready_tongue(app.tongue) {
        app.status = crate::speak::sleeping_reason_tongue(app.tongue);
    } else {
        app.status = format!("tongue: {}", app.tongue.as_str());
        warm_tongue(app);
    }
}

fn warm_tongue(app: &App) {
    let tx = app.tx.clone();
    let tongue = app.tongue;
    std::thread::spawn(move || {
        crate::forge::set_phase("warming tongue");
        let err = crate::speak::ensure_tongue(tongue).err();
        crate::forge::clear_phase();
        let _ = tx.send(UiMsg::Spoke { err });
    });
}

fn spawn_say(app: &App, text: String) {
    let tx = app.tx.clone();
    let tongue = app.tongue;
    std::thread::spawn(move || {
        crate::forge::set_phase("speaking");
        let err = crate::speak::say_tongue(tongue, &text).err();
        crate::forge::clear_phase();
        let _ = tx.send(UiMsg::Spoke { err });
    });
}

fn persist_ui(app: &App) {
    if let Err(error) = prefs::save_ui(&prefs::UiPrefs {
        verbose: app.verbose,
        sparkle: app.sparkle,
        speak_on: app.speak_on,
        speak_kind: app.tongue.as_str().into(),
        rune: app.default_rune,
        build_audit_rounds: app.build_audit_rounds,
        sudo_less_safe: app.sudo_less_safe,
        enhance_image: app.enhance_image,
        art_dir: app.art_dir.clone(),
    }) {
        eprintln!("pixie: could not save house preferences: {error}");
    }
}

pub fn run(verbose: bool, resume: Option<String>, show_door: bool) -> anyhow::Result<()> {
    mind::seed_memory(None)?;
    let (tx, rx) = mpsc::channel();
    let ui = prefs::load_ui()?;
    let (chat, rune_idx, session_id, session_name, door) = if let Some(id) = resume {
        let (c, k) = agent::from_session(&id);
        let idx = runes::all()
            .iter()
            .position(|r| r.key == k)
            .unwrap_or(runes::idx_of(ui.rune));
        let name = mind::load_meta(&id, None).name;
        (c, idx, id, name, false)
    } else {
        let id = chrono_id();
        (
            Vec::new(),
            runes::idx_of(ui.rune),
            id.clone(),
            id,
            show_door,
        )
    };
    let waking = !llm::llm_up();
    let mut app = App {
        chat,
        buf: String::new(),
        cursor: 0,
        rune_idx,
        session_id,
        session_name,
        verbose: verbose || ui.verbose,
        sparkle: ui.sparkle,
        speak_on: ui.speak_on,
        tongue: crate::speak::Tongue::from_str(&ui.speak_kind),
        default_rune: ui.rune,
        build_audit_rounds: ui.build_audit_rounds,
        sudo_less_safe: ui.sudo_less_safe,
        enhance_image: ui.enhance_image,
        art_dir: ui.art_dir.clone(),
        help: false,
        busy: false,
        waking,
        splash: true,
        door,
        options: false,
        opt_idx: 0,
        opt_hits: Vec::new(),
        mouth: prefs::pixie_mouth(),
        mouth_note: String::new(),
        mouth_busy: false,
        compacting: false,
        sudo_buf: None,
        sudo_spin: 0,
        wake_err: None,
        wake_retry: Instant::now(),
        mouth_role: "chat".into(),
        delete_ask: false,
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
        session_menu: false,
        session_idx: 0,
        session_rows: Vec::new(),
        rename: None,
        dash_hits: Vec::new(),
        dash_click: None,
        status: String::new(),
        gen: 0,
        born: Instant::now(),
        look: None,
        file_sel: 0,
        file_hits: Vec::new(),
        pending_editor: None,
        pending_picker: false,
        pending_files: Vec::new(),
        kitty_frame: HashMap::new(),
        kitty_sent: HashMap::new(),
    };
    if app.speak_on {
        warm_tongue(&app);
    }
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
        privilege::idle_tick();
        if app.status.starts_with("sudo setting up") {
            let sp = ["|", "/", "-", "\\"][app.sudo_spin as usize % 4];
            app.sudo_spin = app.sudo_spin.wrapping_add(1);
            app.status = format!("sudo setting up {sp}");
        }
        if app.splash
            && app.wake_err.is_some()
            && !app.waking
            && app.wake_retry.elapsed() >= Duration::from_secs(10)
        {
            retry_wake(&mut app);
        }
        if app.splash && app.waking {
            // star keeps turning until menagerie ensure succeeds
        } else if app.splash && app.wake_err.is_some() {
            // loading / error — do not open the dashboard
        } else if app.splash && !app.waking && app.born.elapsed() >= SPLASH_MIN {
            app.splash = false;
            if app.door {
                open_dashboard(&mut app);
            }
        }
        app.kitty_frame.clear();
        term.draw(|f| draw(f, &mut app))?;
        if kitty::is_terminal() {
            kitty::sync(&mut io::stdout(), &app.kitty_frame, &mut app.kitty_sent);
        }
        if let Some(path) = app.pending_editor.take() {
            if kitty::is_terminal() {
                kitty::clear(&mut io::stdout(), &mut app.kitty_sent);
            }
            suspend_editor(&mut term, &path)?;
        }
        if app.pending_picker {
            app.pending_picker = false;
            if kitty::is_terminal() {
                kitty::clear(&mut io::stdout(), &mut app.kitty_sent);
            }
            suspend_picker(&mut term, &mut app)?;
        }
        let timeout = Duration::from_millis(50).saturating_sub(last_tick.elapsed());
        if event::poll(timeout)? {
            match event::read()? {
                CEvent::Key(k) => {
                    if k.kind != KeyEventKind::Press {
                        continue;
                    }
                    match handle_key(&mut app, k)? {
                        KeyOut::Quit => break,
                        KeyOut::Continue => {}
                    }
                }
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
                        if app.door && !app.options && !app.session_menu {
                            handle_dash_click(&mut app, col, row);
                        }
                        if app.options {
                            for (i, rect) in app.opt_hits.iter().enumerate() {
                                if col >= rect.x
                                    && col < rect.x.saturating_add(rect.width)
                                    && row >= rect.y
                                    && row < rect.y.saturating_add(rect.height)
                                {
                                    app.opt_idx = i;
                                    activate_option(&mut app);
                                    break;
                                }
                            }
                        }
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
                    if app.confirm.peek().is_none()
                        && !app.door
                        && !app.options
                        && !app.session_menu
                    {
                        if let Some(a) = attach::from_paste(&s) {
                            let name = pretty_path(&a.path);
                            app.status = format!("clipped {name}");
                            app.pending_files.push(a);
                        } else {
                            insert_text(&mut app.buf, &mut app.cursor, &sanitize_input(&s));
                        }
                    } else if let Some(buf) = app.rename.as_mut() {
                        buf.push_str(&sanitize_input(&s));
                    }
                }
                CEvent::Resize(_, _) => {}
                _ => {}
            }
        }
        last_tick = Instant::now();
    }
    if kitty::is_terminal() {
        kitty::clear(&mut io::stdout(), &mut app.kitty_sent);
    }
    disable_raw_mode()?;
    execute!(
        io::stdout(),
        LeaveAlternateScreen,
        event::DisableMouseCapture,
        event::DisableBracketedPaste
    )?;
    compact_on_quit(&app);
    crate::speak::stop_sidecar();
    privilege::disable();
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
            if app.speak_on {
                crate::speak::push_live(app.tongue, &sp.visible);
            }
        }
        UiMsg::Calling(name) => {
            crate::speak::drop_partial();
            app.live = format!("calling {name}…");
        }
        UiMsg::Round { n, cap } => {
            app.phase = format!("round {n}/{cap}");
        }
        UiMsg::Tool {
            title,
            body,
            file,
            name,
            think,
            ok,
        } => {
            if file.is_some() {
                let n = file_card_count(app) + 1;
                app.file_sel = n.saturating_sub(1);
                app.scroll = 0;
            }
            let think = if think.is_empty() {
                std::mem::take(&mut app.live_think)
            } else {
                app.live_think.clear();
                think
            };
            app.chat.push(ChatEntry::Tool {
                name,
                title,
                body,
                file,
                think,
                ok,
            });
            crate::speak::drop_partial();
            app.live.clear();
        }
        UiMsg::Wake { err } => {
            app.waking = false;
            if let Some(e) = err {
                app.wake_err = Some(e.clone());
                app.wake_retry = Instant::now();
                app.status = format!("pixie: {e}");
                app.splash = true;
            } else {
                app.wake_err = None;
                if !app.buf.is_empty() && !app.busy && !app.door && !app.options && !app.splash {
                    let t = std::mem::take(&mut app.buf);
                    send(app, &t);
                }
            }
        }
        UiMsg::SudoReady { err } => {
            if let Some(e) = err {
                app.status = format!("sudo still off · {e}");
            } else {
                app.status = format!("sudo on · {}", privilege::status_label());
            }
        }
        UiMsg::Titled { session, name } => {
            if session != app.session_id {
                mind::set_session_name(&session, &name, None);
                return;
            }
            mind::set_session_name(&app.session_id, &name, None);
            let meta = mind::load_meta(&app.session_id, None);
            if !meta.renamed {
                app.session_name = meta.name;
            }
        }
        UiMsg::Compacted { session, text, at } => {
            if session != app.session_id {
                return;
            }
            app.compacting = false;
            if app.busy || app.chat.len() != at || text.trim().is_empty() {
                if !app.busy && app.status.starts_with("braiding") {
                    app.status.clear();
                }
                return;
            }
            agent::commit_compact(&mut app.chat, &app.session_id, text);
            app.status = "thread compacted".into();
        }
        UiMsg::Blurb { id, text } => {
            mind::set_summary(&id, &text, None);
            if let Some(row) = app.session_rows.iter_mut().find(|r| r.id == id) {
                row.summary = mind::preview_safe(&text);
            }
        }
        UiMsg::Mouth { err } => {
            app.mouth_busy = false;
            app.mouth = prefs::pixie_mouth();
            if let Some(e) = err {
                app.status = format!("pixie: {e}");
            } else {
                app.status = "mouth applied".into();
            }
        }
        UiMsg::Spoke { err } => {
            if let Some(e) = err {
                app.status = e;
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
            let compact_ok = !cancelled && err.is_none();
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
                        from_rune: String::new(),
                        to_rune: String::new(),
                        reason: String::new(),
                    },
                    None,
                );
            } else if !text.trim().is_empty() {
                let speak_text = text.clone();
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
                        from_rune: String::new(),
                        to_rune: String::new(),
                        reason: String::new(),
                    },
                    None,
                );
                if app.speak_on {
                    crate::speak::flush_remainder(app.tongue);
                    if !crate::speak::emitted_this_turn() {
                        spawn_say(app, speak_text);
                    }
                }
            }
            app.live.clear();
            app.phase.clear();
            app.scroll = 0;
            if wrote_plan {
                app.plan_menu = true;
                app.plan_idx = 0;
            }
            if compact_ok && !app.compacting && agent::should_compact(&app.chat) {
                maybe_compact(app);
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
    if app.splash {
        match k.code {
            KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                return Ok(KeyOut::Quit);
            }
            KeyCode::Char('q') if ctrl(k) => return Ok(KeyOut::Quit),
            _ => {}
        }
        return Ok(KeyOut::Continue);
    }
    if app.rename.is_some() {
        return handle_rename_key(app, k);
    }
    if app.sudo_buf.is_some() {
        return handle_sudo_prompt(app, k);
    }
    if app.delete_ask {
        match k.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                app.delete_ask = false;
                delete_selected(app);
            }
            _ => app.delete_ask = false,
        }
        return Ok(KeyOut::Continue);
    }
    if app.options {
        return handle_options_key(app, k);
    }
    if app.session_menu {
        match k.code {
            KeyCode::Esc => app.session_menu = false,
            KeyCode::Char('j') | KeyCode::Down => {
                let n = app.session_rows.len().max(1);
                app.session_idx = (app.session_idx + 1) % n;
            }
            KeyCode::Char('k') | KeyCode::Up => {
                let n = app.session_rows.len().max(1);
                app.session_idx = (app.session_idx + n.saturating_sub(1)) % n;
            }
            KeyCode::Enter => {
                if let Some(row) = app.session_rows.get(app.session_idx).cloned() {
                    load_session(app, &row.id);
                    app.door = false;
                } else if app.session_rows.is_empty() {
                    new_session(app);
                    app.door = false;
                }
                app.session_menu = false;
            }
            KeyCode::Char('n') if !ctrl(k) => {
                new_session(app);
                app.door = false;
                app.session_menu = false;
            }
            KeyCode::Char('r') if !ctrl(k) => {
                if let Some(row) = app.session_rows.get(app.session_idx) {
                    app.rename = Some(row.name.clone());
                }
            }
            KeyCode::Char('d') if !ctrl(k) => {
                if app.session_rows.get(app.session_idx).is_some() {
                    app.delete_ask = true;
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
    if app.door {
        return handle_door_key(app, k);
    }
    if app.look.is_some() {
        match k.code {
            KeyCode::Esc => app.look = None,
            KeyCode::Char('o') if ctrl(k) => request_open(app),
            KeyCode::Char('p') if ctrl(k) => {
                cycle_file(app, -1);
                open_look(app);
            }
            KeyCode::Char('n') if ctrl(k) => {
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
                        let from = app.rune_key();
                        app.rune_idx = runes::all()
                            .iter()
                            .position(|r| r.key == RuneKey::Build)
                            .unwrap_or(app.rune_idx);
                        let to = app.rune_key();
                        if from != to {
                            let plan_summary = mind::load_plan(&app.session_id, 1200);
                            let text = if plan_summary.is_empty() {
                                "Switched modes: Plan → Build (plan approved). Implement the active plan in ~/pixie.".to_string()
                            } else {
                                format!(
                                    "Switched modes: Plan → Build (plan approved). Active plan:\n{}",
                                    plan_summary
                                )
                            };
                            mind::append_transition(
                                &app.session_id,
                                from.as_str(),
                                to.as_str(),
                                "plan_approved",
                                &text,
                                None,
                            );
                            app.status = "Plan → Build (approved)".into();
                        }
                        send(
                            app,
                            "Implement the active plan in ~/pixie. Do not write ~/.config/plan. Follow the cached plan.",
                        );
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

    if crate::grove_keys::is_cycle_next(k.code, k.modifiers) {
        let from = app.rune_key();
        app.rune_idx = runes::next_idx(app.rune_idx);
        let to = app.rune_key();
        if from != to {
            let from_name = runes::all()
                .iter()
                .find(|r| r.key == from)
                .map(|r| r.name)
                .unwrap_or(from.as_str());
            let to_name = runes::all()
                .iter()
                .find(|r| r.key == to)
                .map(|r| r.name)
                .unwrap_or(to.as_str());
            mind::append_transition(
                &app.session_id,
                from.as_str(),
                to.as_str(),
                "user_cycled",
                &format!("Switched modes: {from_name} → {to_name} (Tab/Shift+Tab)."),
                None,
            );
            app.status = format!("switched {from_name} → {to_name}");
            hop_role_for_rune(app);
        }
        return Ok(KeyOut::Continue);
    }
    if crate::grove_keys::is_cycle_previous(k.code, k.modifiers) {
        let from = app.rune_key();
        app.rune_idx = runes::prev_idx(app.rune_idx);
        let to = app.rune_key();
        if from != to {
            let from_name = runes::all()
                .iter()
                .find(|r| r.key == from)
                .map(|r| r.name)
                .unwrap_or(from.as_str());
            let to_name = runes::all()
                .iter()
                .find(|r| r.key == to)
                .map(|r| r.name)
                .unwrap_or(to.as_str());
            mind::append_transition(
                &app.session_id,
                from.as_str(),
                to.as_str(),
                "user_cycled",
                &format!("Switched modes: {from_name} → {to_name} (Shift+Tab)."),
                None,
            );
            app.status = format!("switched {from_name} → {to_name}");
            hop_role_for_rune(app);
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
        KeyCode::Char('q') if ctrl(k) => {
            if app.busy {
                app.abort.cancel();
            }
            return Ok(KeyOut::Quit);
        }
        KeyCode::Esc => {
            if app.help {
                app.help = false;
            } else if !app.pending_files.is_empty() && app.buf.is_empty() {
                app.pending_files.clear();
                app.status = "clips dropped".into();
            } else if app.buf.is_empty() {
                if app.busy {
                    app.abort.cancel();
                } else {
                    open_dashboard(app);
                }
            } else {
                app.buf.clear();
                app.cursor = 0;
            }
        }
        KeyCode::Char('v') if ctrl(k) => {
            app.verbose = !app.verbose;
            persist_ui(app);
        }
        KeyCode::Char('v')
            if k.modifiers
                .intersects(KeyModifiers::ALT | KeyModifiers::SUPER) =>
        {
            toggle_speak(app);
        }
        KeyCode::Char('h') if ctrl(k) => app.help = !app.help,
        KeyCode::Char('s') if ctrl(k) && !app.busy => {
            app.plan_menu = false;
            app.session_rows = mind::session_previews(None);
            app.session_idx = 0;
            app.session_menu = true;
        }
        KeyCode::Char('r') if k.modifiers.contains(KeyModifiers::CONTROL) => {
            app.rune_idx = runes::next_idx(app.rune_idx);
        }
        KeyCode::Char('p') if ctrl(k) => cycle_file(app, -1),
        KeyCode::Char('n') if ctrl(k) => cycle_file(app, 1),
        KeyCode::Char('l') if ctrl(k) => open_look(app),
        KeyCode::Char('o') if ctrl(k) => request_open(app),
        KeyCode::Char('u') if ctrl(k) && !app.busy => {
            app.pending_picker = true;
        }
        KeyCode::Enter => {
            if k.modifiers.contains(KeyModifiers::SHIFT) {
                insert_text(&mut app.buf, &mut app.cursor, "\n");
            } else {
                let text = app.buf.trim().to_string();
                if !text.is_empty() || !app.pending_files.is_empty() {
                    app.buf.clear();
                    app.cursor = 0;
                    send(app, &text);
                }
            }
        }
        KeyCode::Backspace => {
            if app.buf.is_empty() && !app.pending_files.is_empty() {
                if let Some(a) = app.pending_files.pop() {
                    app.status = format!("dropped {}", pretty_path(&a.path));
                }
            } else {
                delete_before(&mut app.buf, &mut app.cursor);
            }
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
            } else if k.modifiers.contains(KeyModifiers::SUPER) {
            } else {
                insert_text(
                    &mut app.buf,
                    &mut app.cursor,
                    &sanitize_input(&c.to_string()),
                );
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

fn ctrl(k: KeyEvent) -> bool {
    k.modifiers.contains(KeyModifiers::CONTROL)
}

fn handle_door_key(app: &mut App, k: KeyEvent) -> anyhow::Result<KeyOut> {
    match k.code {
        KeyCode::Esc => return Ok(KeyOut::Quit),
        KeyCode::Char('q') if ctrl(k) => return Ok(KeyOut::Quit),
        KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => {
            return Ok(KeyOut::Quit);
        }
        KeyCode::Char('j') | KeyCode::Down => {
            let n = app.session_rows.len().max(1);
            app.session_idx = (app.session_idx + 1) % n;
        }
        KeyCode::Char('k') | KeyCode::Up => {
            let n = app.session_rows.len().max(1);
            app.session_idx = (app.session_idx + n.saturating_sub(1)) % n;
        }
        KeyCode::Char('n') if !ctrl(k) => {
            new_session(app);
            app.door = false;
        }
        KeyCode::Char('o') if !ctrl(k) => {
            app.mouth = prefs::pixie_mouth();
            app.opt_idx = 0;
            app.options = true;
        }
        KeyCode::Enter => {
            if let Some(row) = app.session_rows.get(app.session_idx).cloned() {
                load_session(app, &row.id);
                app.door = false;
            } else {
                new_session(app);
                app.door = false;
            }
        }
        KeyCode::Char('r') if !ctrl(k) => {
            if let Some(row) = app.session_rows.get(app.session_idx) {
                app.rename = Some(row.name.clone());
            }
        }
        KeyCode::Char('d') if !ctrl(k) && app.session_rows.get(app.session_idx).is_some() => {
            app.delete_ask = true;
        }
        _ => {}
    }
    Ok(KeyOut::Continue)
}

fn handle_rename_key(app: &mut App, k: KeyEvent) -> anyhow::Result<KeyOut> {
    match k.code {
        KeyCode::Esc => app.rename = None,
        KeyCode::Enter => {
            if let (Some(name), Some(row)) = (
                app.rename.take(),
                app.session_rows.get(app.session_idx).cloned(),
            ) {
                let id = row.id;
                mind::rename_session(&id, &name, None);
                if id == app.session_id {
                    app.session_name = mind::load_meta(&id, None).name;
                }
                app.session_rows = mind::session_previews(None);
                if let Some(i) = app.session_rows.iter().position(|r| r.id == id) {
                    app.session_idx = i;
                }
            }
        }
        KeyCode::Backspace => {
            if let Some(buf) = app.rename.as_mut() {
                buf.pop();
            }
        }
        KeyCode::Char(c) if !k.modifiers.contains(KeyModifiers::CONTROL) => {
            if let Some(buf) = app.rename.as_mut() {
                if buf.chars().count() < 40 && !c.is_control() {
                    buf.push(c);
                }
            }
        }
        _ => {}
    }
    Ok(KeyOut::Continue)
}

fn handle_sudo_prompt(app: &mut App, k: KeyEvent) -> anyhow::Result<KeyOut> {
    match k.code {
        KeyCode::Esc => app.sudo_buf = None,
        KeyCode::Enter => {
            if let Some(pw) = app.sudo_buf.take() {
                if pw.is_empty() {
                    app.status = "sudo still off".into();
                } else {
                    app.status = "sudo setting up |".into();
                    app.sudo_spin = 0;
                    let less = app.sudo_less_safe;
                    let tx = app.tx.clone();
                    std::thread::spawn(move || {
                        let r = if less {
                            privilege::enable_less_safe(pw)
                        } else {
                            privilege::enable_sitting(pw)
                        };
                        let _ = tx.send(UiMsg::SudoReady {
                            err: r.err().map(|e| e.to_string()),
                        });
                    });
                }
            }
        }
        KeyCode::Backspace => {
            if let Some(buf) = app.sudo_buf.as_mut() {
                buf.pop();
            }
        }
        KeyCode::Char(c) if !k.modifiers.contains(KeyModifiers::CONTROL) && !c.is_control() => {
            if let Some(buf) = app.sudo_buf.as_mut() {
                if buf.len() < 128 {
                    buf.push(c);
                }
            }
        }
        _ => {}
    }
    Ok(KeyOut::Continue)
}

fn handle_options_key(app: &mut App, k: KeyEvent) -> anyhow::Result<KeyOut> {
    const N: usize = 16;
    match k.code {
        KeyCode::Esc => {
            app.options = false;
        }
        KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => {
            return Ok(KeyOut::Quit);
        }
        KeyCode::Char('q') if ctrl(k) => return Ok(KeyOut::Quit),
        KeyCode::Char('j') | KeyCode::Down => app.opt_idx = (app.opt_idx + 1) % N,
        KeyCode::Char('k') | KeyCode::Up => app.opt_idx = (app.opt_idx + N - 1) % N,
        KeyCode::Char('o') if ctrl(k) => open_persona(app),
        KeyCode::Left => cycle_option(app, -1),
        KeyCode::Right => cycle_option(app, 1),
        KeyCode::Enter => activate_option(app),
        _ => {}
    }
    Ok(KeyOut::Continue)
}

fn cycle_option(app: &mut App, dir: i32) {
    match app.opt_idx {
        0 => {
            const ROLES: [&str; 3] = ["chat", "code", "vision"];
            let i = ROLES.iter().position(|r| *r == app.mouth_role).unwrap_or(0) as i32;
            let j = (i + dir).rem_euclid(3) as usize;
            app.mouth_role = ROLES[j].to_string();
        }
        1 => {
            let names = prefs::chat_model_names();
            if names.is_empty() {
                return;
            }
            let i = names
                .iter()
                .position(|n| n == &app.mouth.model)
                .unwrap_or(0);
            let n = names.len() as i32;
            let j = (i as i32 + dir).rem_euclid(n) as usize;
            app.mouth.model = names[j].clone();
        }
        2 => {
            app.mouth.backend = if app.mouth.backend == "gpu" {
                "cpu".into()
            } else {
                "gpu".into()
            };
        }
        3 => {
            app.mouth.ctx = if app.mouth.ctx >= 16384 { 8192 } else { 16384 };
        }
        5 => {
            app.sparkle = !app.sparkle;
            persist_ui(app);
        }
        6 => {
            app.verbose = !app.verbose;
            persist_ui(app);
        }
        7 => cycle_voice(app, dir),
        8 => {
            app.default_rune = app.default_rune.next();
            if app.door {
                app.rune_idx = runes::idx_of(app.default_rune);
            }
            persist_ui(app);
        }
        11 => {
            app.enhance_image = !app.enhance_image;
            persist_ui(app);
        }
        12 => cycle_art_dir(app, dir),
        13 => {
            app.build_audit_rounds = !app.build_audit_rounds;
            persist_ui(app);
        }
        14 => {
            app.sudo_less_safe = !app.sudo_less_safe;
            persist_ui(app);
        }
        15 => toggle_sudo(app),
        _ => {}
    }
}

fn activate_option(app: &mut App) {
    match app.opt_idx {
        0..=3 => apply_mouth_async(app),
        4 => open_persona(app),
        5 => {
            app.sparkle = !app.sparkle;
            persist_ui(app);
        }
        6 => {
            app.verbose = !app.verbose;
            persist_ui(app);
        }
        7 => cycle_voice(app, 1),
        8 => {
            app.default_rune = app.default_rune.next();
            if app.door {
                app.rune_idx = runes::idx_of(app.default_rune);
            }
            persist_ui(app);
        }
        9 => {
            app.status = if prefs::eye_asleep() {
                "eye is sleeping — no mmproj on disk".into()
            } else {
                "look hops the GPU (park 8B, VL 7B, restore Chat)".into()
            };
        }
        10 => app.status = "Imp hops the GPU; generate_image parks Pixie first".into(),
        11 => {
            app.enhance_image = !app.enhance_image;
            persist_ui(app);
            app.status = format!(
                "enhance wish: {}",
                if app.enhance_image { "on" } else { "off" }
            );
        }
        12 => {
            cycle_art_dir(app, 1);
            app.status = format!("art folder: {}", app.art_dir);
        }
        13 => {
            app.build_audit_rounds = !app.build_audit_rounds;
            persist_ui(app);
            app.status = format!(
                "build audit rounds: {}",
                if app.build_audit_rounds { "on" } else { "off" }
            );
        }
        14 => {
            app.sudo_less_safe = !app.sudo_less_safe;
            persist_ui(app);
            app.status = format!(
                "sudo less-safe: {}",
                if app.sudo_less_safe { "on" } else { "off" }
            );
        }
        15 => toggle_sudo(app),
        _ => {}
    }
}

fn cycle_art_dir(app: &mut App, dir: i32) {
    let mut choices = vec![
        "~/pixie_art".to_string(),
        "~/Pictures".to_string(),
        "~/imp".to_string(),
    ];
    if !choices.iter().any(|c| c == &app.art_dir) {
        choices.insert(0, app.art_dir.clone());
    }
    let i = choices
        .iter()
        .position(|c| c == &app.art_dir)
        .unwrap_or(0) as i32;
    let n = choices.len() as i32;
    let j = (i + dir).rem_euclid(n) as usize;
    app.art_dir = choices[j].clone();
    persist_ui(app);
}

fn toggle_sudo(app: &mut App) {
    if privilege::is_on() {
        privilege::disable();
        app.status = "sudo off".into();
        return;
    }
    if privilege::ticket_ok() {
        privilege::enable_ticket();
        app.status = "sudo on · ticket".into();
        return;
    }
    app.sudo_buf = Some(String::new());
    app.status = "sudo: enter password (sitting). less-safe available via code/config".into();
}

fn open_persona(app: &mut App) {
    let p = paths::persona_path();
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    if !p.is_file() {
        let _ = std::fs::write(
            &p,
            "# Pixie persona overlay\n# Empty file uses the compiled PERSONA.\n",
        );
    }
    app.pending_editor = Some(p);
}

fn hop_role_for_rune(app: &mut App) {
    let want = if app.rune_key() == RuneKey::Build {
        "code"
    } else {
        "chat"
    };
    if app.mouth_role == want {
        return;
    }
    app.mouth_role = want.to_string();
    apply_mouth_async(app);
}

fn apply_mouth_async(app: &mut App) {
    if app.mouth_busy || app.busy {
        app.status = "pixie: mouth is busy".into();
        return;
    }
    app.mouth_busy = true;
    app.status = "applying mouth…".into();
    let draft = app.mouth.clone();
    let slot = app.mouth_role.clone();
    let old = prefs::pixie_mouth();
    let tx = app.tx.clone();
    std::thread::spawn(move || {
        let err = match prefs::apply_mouth_slot(&slot, &draft.model, &draft.backend, draft.ctx) {
            Ok(()) => None,
            Err(e) => {
                let _ = prefs::apply_mouth_slot("chat", &old.model, &old.backend, old.ctx);
                Some(e)
            }
        };
        let _ = tx.send(UiMsg::Mouth { err });
    });
}

fn maybe_compact(app: &mut App) {
    if app.compacting {
        return;
    }
    app.compacting = true;
    app.status = "braiding a compact…".into();
    let at = app.chat.len();
    let session = app.session_id.clone();
    let owned = app.chat.clone();
    let tx = app.tx.clone();
    std::thread::spawn(move || {
        let text = agent::request_compact(&owned).unwrap_or_default();
        let _ = tx.send(UiMsg::Compacted { session, text, at });
    });
}

fn open_dashboard(app: &mut App) {
    app.door = true;
    app.options = false;
    app.session_menu = false;
    app.plan_menu = false;
    app.look = None;
    app.help = false;
    app.delete_ask = false;
    app.busy = false;
    app.live.clear();
    app.live_think.clear();
    mind::hydrate_compact_blurbs(None);
    app.session_rows = mind::session_previews(None);
    if app.session_idx >= app.session_rows.len() {
        app.session_idx = 0;
    }
    refresh_mouth_note(app);
    request_blurbs(app);
}

fn refresh_mouth_note(app: &mut App) {
    app.mouth = prefs::pixie_mouth();
    let p = if llm::port().is_some_and(llm::mouth_up) {
        "up"
    } else {
        "asleep"
    };
    let k = if llm::mouth_up(crate::voice::voice_port()) {
        "up"
    } else {
        "asleep"
    };
    let eye = if prefs::eye_asleep() {
        "eye asleep"
    } else {
        "eye"
    };
    let tongue = if crate::speak::ready_tongue(crate::speak::Tongue::Design)
        && crate::speak::ready_tongue(crate::speak::Tongue::Vivian)
    {
        "tongue vivian+design"
    } else if crate::speak::ready_tongue(crate::speak::Tongue::Vivian) {
        "tongue vivian"
    } else if crate::speak::ready_tongue(crate::speak::Tongue::Design) {
        "tongue design"
    } else {
        "tongue asleep"
    };
    app.mouth_note = format!(
        "pixie {p} {} {} · kur {k} · {eye} · {tongue}",
        app.mouth.backend, app.mouth.ctx
    );
}

fn request_blurbs(app: &mut App) {
    let ids: Vec<String> = app
        .session_rows
        .iter()
        .filter(|r| mind::needs_summary(&r.id, None))
        .take(5)
        .map(|r| r.id.clone())
        .collect();
    if ids.is_empty() {
        return;
    }
    let tx = app.tx.clone();
    std::thread::spawn(move || {
        for id in ids {
            let prompts = mind::user_texts(&id, None, 10);
            let text = crate::voice::summarize_thread(&prompts);
            if !text.is_empty() {
                let _ = tx.send(UiMsg::Blurb { id, text });
            }
        }
    });
}

fn wake_if_needed(app: &mut App) {
    if app.waking || llm::port().is_some_and(llm::mouth_up) {
        return;
    }
    retry_wake(app);
}

fn retry_wake(app: &mut App) {
    if app.waking {
        return;
    }
    app.waking = true;
    app.splash = true;
    app.wake_retry = Instant::now();
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

fn delete_selected(app: &mut App) {
    let Some(row) = app.session_rows.get(app.session_idx).cloned() else {
        return;
    };
    if !mind::delete_session(&row.id, None) {
        app.status = "pixie: could not delete that thread".into();
        return;
    }
    if app.session_id == row.id {
        new_session(app);
        app.door = true;
        app.busy = false;
    }
    app.session_rows = mind::session_previews(None);
    if app.session_idx >= app.session_rows.len() {
        app.session_idx = app.session_rows.len().saturating_sub(1);
    }
}

fn handle_dash_click(app: &mut App, col: u16, row: u16) {
    for (rect, hit) in &app.dash_hits {
        if col >= rect.x
            && col < rect.x.saturating_add(rect.width)
            && row >= rect.y
            && row < rect.y.saturating_add(rect.height)
        {
            match *hit {
                DashHit::New => {
                    new_session(app);
                    app.door = false;
                }
                DashHit::Options => {
                    app.mouth = prefs::pixie_mouth();
                    app.opt_idx = 0;
                    app.options = true;
                }
                DashHit::Session(i) => {
                    app.session_idx = i;
                    let now = Instant::now();
                    let dbl = app
                        .dash_click
                        .map(|(t, j)| j == i && now.duration_since(t) < Duration::from_millis(400))
                        .unwrap_or(false);
                    app.dash_click = Some((now, i));
                    if dbl {
                        if let Some(s) = app.session_rows.get(i).cloned() {
                            load_session(app, &s.id);
                            app.door = false;
                        }
                    }
                }
            }
            return;
        }
    }
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
    let files = std::mem::take(&mut app.pending_files);
    let args = if files.is_empty() {
        None
    } else {
        Some(json!({
            "files": files.iter().map(|a| a.to_json()).collect::<Vec<_>>()
        }))
    };
    app.chat.push(ChatEntry::User {
        text: text.to_string(),
        files,
    });
    mind::append_session(
        &app.session_id,
        &SessionRec {
            ts: mind::now_ts(),
            role: "user".into(),
            rune: app.rune_key().as_str().into(),
            text: text.to_string(),
            tool: String::new(),
            args,
            result: None,
            raw: String::new(),
            think: String::new(),
            cancelled: false,
            from_rune: String::new(),
            to_rune: String::new(),
            reason: String::new(),
        },
        None,
    );
    if mind::should_retitle(&app.session_id, None) {
        mind::bump_named_from(&app.session_id, None);
        let prompts = mind::user_texts(&app.session_id, None, 10);
        let tx = app.tx.clone();
        let sid = app.session_id.clone();
        std::thread::spawn(move || {
            let name = crate::voice::title_thread(&prompts);
            if !name.is_empty() {
                let _ = tx.send(UiMsg::Titled { session: sid, name });
            }
        });
    }
    app.busy = true;
    app.live.clear();
    app.live_think.clear();
    if app.speak_on {
        crate::speak::begin_turn();
    }
    app.scroll = 0;
    let rune = runes::all()[app.rune_idx].clone();
    let session = app.session_id.clone();
    let verbose = app.verbose;
    let sparkle = app.sparkle;
    let abort = app.abort.clone();
    let confirm = app.confirm.clone();
    let tx = app.tx.clone();
    let mut owned = app.chat.clone();
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
                    title,
                    body,
                    file,
                    name,
                    think,
                    ok,
                    ..
                } => {
                    let _ = tx.send(UiMsg::Tool {
                        title,
                        body,
                        file,
                        name,
                        think,
                        ok,
                    });
                }
                Event::ToolStart { name, .. } => {
                    let _ = tx.send(UiMsg::Calling(name));
                }
            },
        );
        let mut text = out.text;
        if sparkle && !out.cancelled && out.err.is_none() && !text.trim().is_empty() {
            text = crate::voice::prepend("final answer for the human", &text);
        }
        let _ = tx.send(UiMsg::Done {
            gen,
            text,
            think: out.think,
            cancelled: out.cancelled,
            err: out.err,
            wrote_plan: out.wrote_plan,
        });
    });
}

fn file_card_count(app: &App) -> usize {
    file_cards(app).len()
}

fn file_cards(app: &App) -> Vec<FileTouch> {
    let mut out = Vec::new();
    for e in &app.chat {
        match e {
            ChatEntry::User { files, .. } => {
                for a in files {
                    out.push(a.as_touch());
                }
            }
            ChatEntry::Tool { file: Some(f), .. } => out.push(f.clone()),
            _ => {}
        }
    }
    out
}

fn current_touch(app: &App) -> Option<FileTouch> {
    let cards = file_cards(app);
    if cards.is_empty() {
        return None;
    }
    let want = app.file_sel.min(cards.len() - 1);
    cards.into_iter().nth(want)
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
    let image = touch.kind == diff::FileKind::Image;
    let lines = look_lines(&touch);
    app.look = Some(LookState {
        path,
        lines,
        scroll: 0,
        image,
    });
}

fn look_lines(touch: &FileTouch) -> Vec<(Op, String)> {
    if touch.kind == diff::FileKind::Image {
        return vec![(Op::Eq, format!("picture · {}", pretty_path(&touch.path)))];
    }
    if touch.binary {
        return vec![(Op::Eq, "not a text file".into())];
    }
    if !touch.lines.is_empty() {
        return touch.lines.iter().map(|l| (l.op, l.text.clone())).collect();
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
    let argv = open_argv(path);
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

fn suspend_picker<B: ratatui::backend::Backend>(
    term: &mut Terminal<B>,
    app: &mut App,
) -> anyhow::Result<()> {
    disable_raw_mode()?;
    execute!(
        io::stdout(),
        LeaveAlternateScreen,
        event::DisableMouseCapture,
        event::DisableBracketedPaste
    )?;
    let _ = term.show_cursor();
    let out = std::env::temp_dir().join(format!("pixie-pick-{}.path", std::process::id()));
    let _ = std::fs::remove_file(&out);
    let start = attach::pick_start(&app.pending_files);
    let spellbook = attach::spellbook_bin();
    let status = std::process::Command::new(&spellbook)
        .arg("--pick")
        .arg("--output")
        .arg(&out)
        .arg(&start)
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .status();
    enable_raw_mode()?;
    execute!(
        io::stdout(),
        EnterAlternateScreen,
        event::EnableMouseCapture,
        event::EnableBracketedPaste
    )?;
    term.clear()?;
    match status {
        Ok(s) if s.success() || out.is_file() => {
            if let Ok(t) = std::fs::read_to_string(&out) {
                let p = PathBuf::from(t.trim());
                match Attach::from_path(&p) {
                    Ok(a) => {
                        app.status = format!("clipped {}", pretty_path(&a.path));
                        app.pending_files.push(a);
                    }
                    Err(e) => app.status = format!("pixie: {e}"),
                }
            }
        }
        Ok(_) => app.status = "clip cancelled".into(),
        Err(e) => {
            app.status = format!("pixie: spellbook pick failed ({e})");
        }
    }
    let _ = std::fs::remove_file(&out);
    Ok(())
}

fn editor_argv() -> Vec<String> {
    let raw = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "nano".into());
    mind::split_argv(&raw).unwrap_or_else(|_| vec![raw])
}

fn is_picture(path: &Path) -> bool {
    match path
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase())
    {
        Some(ext) => matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "webp" | "gif"),
        None => false,
    }
}

fn first_on_path(names: &[&str]) -> Option<String> {
    let path = std::env::var("PATH").unwrap_or_default();
    for name in names {
        for dir in path.split(':') {
            let p = Path::new(dir).join(name);
            if p.is_file() {
                return Some((*name).to_string());
            }
        }
    }
    None
}

fn open_argv(path: &Path) -> Vec<String> {
    if is_picture(path) {
        if let Some(bin) = first_on_path(&["imv", "imv-x11", "feh", "xdg-open"]) {
            return vec![bin];
        }
    }
    editor_argv()
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
        return (
            format!("{spin}  {}", tools::confirm_line(&name, &args)),
            false,
        );
    }
    if !app.pending_files.is_empty() && !app.busy {
        let chips: String = app
            .pending_files
            .iter()
            .map(|a| format!("[{} {}]", a.kind.as_str(), pretty_path(&a.path)))
            .collect::<Vec<_>>()
            .join(" ");
        if app.buf.is_empty() {
            return (
                format!("{chips}\n✦  enter send · C-u more · backspace drops"),
                true,
            );
        }
        return (format!("{chips}\n✦ {}", app.buf), true);
    }
    if !app.buf.is_empty() {
        return (format!("✦ {}", app.buf), true);
    }
    if app.splash || app.waking {
        return (format!("{spin}  {wait}"), false);
    }
    if app.sudo_buf.is_some() {
        let n = app
            .sudo_buf
            .as_ref()
            .map(|s| s.chars().count())
            .unwrap_or(0);
        let mask = "•".repeat(n);
        return (format!("sudo sitting · {mask}_  enter · esc"), false);
    }
    if app.delete_ask {
        return ("delete this thread?  [y/N]".into(), false);
    }
    if app.rename.is_some() {
        return ("rename · enter save · esc".into(), false);
    }
    if app.options {
        return (
            "arrows · enter apply · C-o persona · esc back".into(),
            false,
        );
    }
    if app.door && !app.session_menu {
        return (
            "enter open · n new · o options · r rename · d delete · esc quit".into(),
            false,
        );
    }
    if app.session_menu {
        return (
            "enter load · n new · r rename · d delete · esc back".into(),
            false,
        );
    }
    if let Some(p) = crate::forge::phase_status(spin) {
        if app.busy {
            return (format!("{p}  (type + enter barge-in)"), false);
        }
        return (p, false);
    }
    if app.busy {
        let phase = if app.phase.is_empty() {
            wait.to_string()
        } else {
            format!("{} · {wait}", app.phase)
        };
        return (format!("{spin}  {phase}  (type + enter barge-in)"), false);
    }
    let hint = if app.status.is_empty() {
        format!("{rune_desc} · tab rune · C-h help")
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
    let text = sanitize_input(text);
    if text.is_empty() {
        return;
    }
    let i = byte_at(buf, *cursor);
    buf.insert_str(i, &text);
    *cursor = cursor.saturating_add(text.chars().count());
}

/// Drop CSI / SGR mouse dumps so click-drag cannot fill a you-bubble.
fn sanitize_input(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == 0x1b {
            i += 1;
            if i < b.len() && b[i] == b'[' {
                i += 1;
                while i < b.len() && !b[i].is_ascii_alphabetic() && b[i] != b'~' {
                    i += 1;
                }
                if i < b.len() {
                    i += 1;
                }
            }
            continue;
        }
        if b[i] == b'[' && i + 1 < b.len() && b[i + 1] == b'<' {
            let mut j = i + 2;
            while j < b.len() && (b[j].is_ascii_digit() || b[j] == b';') {
                j += 1;
            }
            if j < b.len() && (b[j] == b'M' || b[j] == b'm') {
                i = j + 1;
                continue;
            }
        }
        if b[i] == b'<' && i + 1 < b.len() && b[i + 1].is_ascii_digit() {
            let mut j = i + 1;
            let mut semis = 0u8;
            while j < b.len() && (b[j].is_ascii_digit() || b[j] == b';') {
                if b[j] == b';' {
                    semis += 1;
                }
                j += 1;
            }
            if semis >= 1 && j < b.len() && (b[j] == b'M' || b[j] == b'm') {
                i = j + 1;
                continue;
            }
        }
        let ch = text[i..].chars().next().unwrap();
        if !ch.is_control() || ch == '\n' || ch == '\t' {
            out.push(ch);
        }
        i += ch.len_utf8();
    }
    out
}

fn live_speech(app: &App) -> String {
    mask_live_speech(
        &app.live,
        app.confirm.peek().as_ref().map(|(n, _)| n.as_str()),
    )
}

/// Hide a live JSON tool call. Keep a normal reply even if it mentions a tool.
fn mask_live_speech(live: &str, confirm_name: Option<&str>) -> String {
    if let Some(name) = confirm_name {
        return format!("calling {name}…");
    }
    let vis = crate::think::strip(live);
    let trimmed = vis.trim();
    if trimmed.starts_with("calling ") && trimmed.lines().count() == 1 {
        return vis;
    }
    if trimmed.starts_with('{') || trimmed.starts_with("<tool_call>") {
        if let Some(c) = mind::extract_tool_calls(trimmed, 1).into_iter().next() {
            return format!("calling {}…", c.name);
        }
    }
    vis
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
    place: Option<kitty::Place>,
}

fn draw(f: &mut Frame, app: &mut App) {
    let size = f.area();
    let runes = runes::all();
    let rune = &runes[app.rune_idx];
    let (prompt_inner, editing) = prompt_text(app, rune.desc);
    let extra = if app.help { 6 } else { 0 };
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
    } else if app.look.is_some() {
        draw_look(f, chunks[0], app);
    } else if app.options {
        draw_options_card(f, chunks[0], app);
    } else if app.session_menu {
        draw_session_card(f, chunks[0], app);
    } else if app.door {
        draw_door_card(f, chunks[0], app);
    } else if app.plan_menu {
        draw_plan_card(f, chunks[0], app);
    } else {
        draw_chat(f, chunks[0], app, rune.name);
    }

    if app.help {
        let help = Paragraph::new(
            "enter send · esc clear/cancel · C-q quit · tab / shift-tab runes · C-v verbose · C-h help\n\
             C-s sessions · C-u clip · C-l look · C-o open editor · C-p / C-n files · A-v speak · esc home · C-q quit",
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
    }

    let inner_w = chrome::prompt_inner_width(chunks[2].width);
    let rows = chrome::wrap(&prompt_inner, inner_w);
    let inner_h = chunks[2].height.saturating_sub(2).max(1) as usize;
    let (crow, ccol) = if editing {
        let prefix_chars = prompt_inner
            .rfind('✦')
            .map(|i| prompt_inner[..i].chars().count() + "✦ ".chars().count())
            .unwrap_or_else(|| "✦ ".chars().count());
        let vis = prefix_chars + app.cursor.min(app.buf.chars().count());
        chrome::wrap_cursor(&prompt_inner, inner_w, vis)
    } else {
        (0, 0)
    };
    let start = (crow as usize + 1).saturating_sub(inner_h);
    let shown: Vec<Line> = rows
        .iter()
        .skip(start)
        .take(inner_h)
        .map(|s| Line::from(s.clone()))
        .collect();
    f.render_widget(
        Paragraph::new(shown).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(BLUSH))
                .border_set(chrome::border_set()),
        ),
        chunks[2],
    );
    if editing {
        let row = (crow as usize).saturating_sub(start) as u16;
        let x = chunks[2].x.saturating_add(1).saturating_add(ccol);
        let y = chunks[2].y.saturating_add(1).saturating_add(row);
        if x < chunks[2]
            .x
            .saturating_add(chunks[2].width.saturating_sub(1))
            && y < chunks[2]
                .y
                .saturating_add(chunks[2].height.saturating_sub(1))
        {
            f.set_cursor_position(Position { x, y });
        }
    }

    let pills: Vec<Span> = runes
        .iter()
        .enumerate()
        .flat_map(|(i, r)| {
            let gap = if i == 0 { None } else { Some(Span::raw("  ")) };
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
        if app.verbose {
            "  · C-v verbose"
        } else {
            "  · C-v"
        },
        if app.verbose {
            Style::default().fg(PINK)
        } else {
            Style::default().fg(SILVER)
        },
    ));
    footer.push(Span::styled(
        if app.speak_on {
            "  · A-v speak"
        } else {
            "  · A-v"
        },
        if app.speak_on {
            Style::default().fg(PINK)
        } else {
            Style::default().fg(SILVER)
        },
    ));
    footer.push(Span::styled("  · C-h", Style::default().fg(SILVER)));
    f.render_widget(Paragraph::new(Line::from(footer)), chunks[3]);
}

fn draw_door_card(f: &mut Frame, area: Rect, app: &mut App) {
    app.dash_hits.clear();
    f.render_widget(Clear, area);
    let block = Block::default()
        .title(" ✦ Pixie ✦ ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(PINK))
        .border_set(chrome::border_set());
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.width < 12 || inner.height < 6 {
        return;
    }
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(9), Constraint::Min(16)])
        .split(inner);
    for (i, line) in glyph::SIGIL.iter().enumerate() {
        let y = cols[0].y.saturating_add(i as u16);
        if y >= cols[0].y.saturating_add(cols[0].height) {
            break;
        }
        f.render_widget(
            Paragraph::new(*line).style(Style::default().fg(PINK)),
            Rect {
                x: cols[0].x,
                y,
                width: cols[0].width.min(7),
                height: 1,
            },
        );
    }
    let right = cols[1];
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .split(right);
    let new_style = if app.session_rows.is_empty() {
        Style::default().fg(PINK).add_modifier(Modifier::REVERSED)
    } else {
        Style::default().fg(BLUSH)
    };
    let pills = Line::from(vec![
        Span::styled(" New ", new_style),
        Span::raw("   "),
        Span::styled(" Options ", Style::default().fg(SILVER)),
    ]);
    f.render_widget(Paragraph::new(pills), rows[0]);
    let pill_y = rows[0].y;
    app.dash_hits.push((
        Rect {
            x: rows[0].x,
            y: pill_y,
            width: 5,
            height: 1,
        },
        DashHit::New,
    ));
    app.dash_hits.push((
        Rect {
            x: rows[0].x.saturating_add(8),
            y: pill_y,
            width: 9,
            height: 1,
        },
        DashHit::Options,
    ));

    let list = rows[1];
    if app.session_rows.is_empty() {
        f.render_widget(
            Paragraph::new("(no threads yet — n starts one)").style(Style::default().fg(DIM)),
            list,
        );
    } else {
        let pair_h = 2u16;
        let vis = (list.height / pair_h).max(1) as usize;
        let start = if app.session_idx >= vis {
            app.session_idx + 1 - vis
        } else {
            0
        };
        let mut y = list.y;
        for (i, row) in app.session_rows.iter().enumerate().skip(start) {
            if y.saturating_add(1) >= list.y.saturating_add(list.height) {
                break;
            }
            let sel = i == app.session_idx;
            let mark = if sel { "▸ " } else { "  " };
            let name = if row.name.is_empty() {
                row.id.as_str()
            } else {
                row.name.as_str()
            };
            let age = mind::age_label(row.mtime);
            let head = format!(
                "{mark}{name}  ·  {} you · {} pixie · {age}",
                row.users, row.assistants
            );
            let blurb = if row.summary.is_empty() {
                "(empty)".to_string()
            } else {
                format!("  {}", row.summary)
            };
            let color = if sel { PINK } else { FG };
            f.render_widget(
                Paragraph::new(head).style(Style::default().fg(color)),
                Rect {
                    x: list.x,
                    y,
                    width: list.width,
                    height: 1,
                },
            );
            f.render_widget(
                Paragraph::new(blurb).style(Style::default().fg(DIM)),
                Rect {
                    x: list.x,
                    y: y.saturating_add(1),
                    width: list.width,
                    height: 1,
                },
            );
            app.dash_hits.push((
                Rect {
                    x: list.x,
                    y,
                    width: list.width,
                    height: 2,
                },
                DashHit::Session(i),
            ));
            y = y.saturating_add(pair_h);
        }
    }
    let note = if app.mouth_note.is_empty() {
        "kur CPU · eye asleep"
    } else {
        app.mouth_note.as_str()
    };
    f.render_widget(
        Paragraph::new(note).style(Style::default().fg(SILVER)),
        rows[2],
    );
}

fn draw_options_card(f: &mut Frame, area: Rect, app: &mut App) {
    app.opt_hits.clear();
    let persona = if prefs::persona_text().is_some() {
        "overlay"
    } else {
        "compiled"
    };
    let sparkle = if app.sparkle { "on" } else { "off" };
    let verbose = if app.verbose { "on" } else { "off" };
    let voice = if !app.speak_on {
        "off".to_string()
    } else if crate::speak::ready_tongue(app.tongue) {
        format!("on · {}", app.tongue.as_str())
    } else {
        format!("on · {} sleeping", app.tongue.as_str())
    };
    let eye = if prefs::eye_asleep() {
        "asleep  (no VL GGUF)"
    } else {
        "ready"
    };
    let ctx_s = app.mouth.ctx.to_string();
    let audit = if app.build_audit_rounds { "on" } else { "off" };
    let less_safe = if app.sudo_less_safe { "on" } else { "off" };
    let enhance = if app.enhance_image { "on" } else { "off" };
    let rows = [
        ("Role", app.mouth_role.as_str()),
        ("Model", app.mouth.model.as_str()),
        ("Backend", app.mouth.backend.as_str()),
        ("Context", ctx_s.as_str()),
        ("Persona", persona),
        ("Sparkle", sparkle),
        ("Verbose", verbose),
        ("Voice", voice.as_str()),
        ("Rune", app.default_rune.as_str()),
        ("Eye", eye),
        ("Imp", "hop · one GPU tenant"),
        ("Enhance", enhance),
        ("Art", app.art_dir.as_str()),
        ("Audit", audit),
        ("LessSafe", less_safe),
        ("Sudo", privilege::status_label()),
    ];
    let mut t = String::from(
        "mouse/sliders-like ◀▶ or click  ·  enter toggle/apply\nSudo: off/ticket/sitting (less-safe optional)\n\n",
    );
    for (i, (label, val)) in rows.iter().enumerate() {
        let mark = if i == app.opt_idx { "▸ " } else { "  " };
        t.push_str(&format!("{mark}{label:<10}  {val}\n"));
    }
    if !app.status.is_empty() {
        t.push('\n');
        t.push_str(&app.status);
    }
    f.render_widget(Clear, area);
    let block_area = area;
    let inner = Rect {
        x: block_area.x + 1,
        y: block_area.y + 3,
        width: block_area.width.saturating_sub(2),
        height: rows.len() as u16,
    };
    for i in 0..rows.len() {
        let y = inner.y + i as u16;
        if y < inner.y + inner.height {
            app.opt_hits.push(Rect {
                x: inner.x,
                y,
                width: inner.width,
                height: 1,
            });
        }
    }
    f.render_widget(
        Paragraph::new(t)
            .wrap(Wrap { trim: false })
            .style(Style::default().fg(FG))
            .block(
                Block::default()
                    .title(" options ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(PINK))
                    .border_set(chrome::border_set()),
            ),
        area,
    );
}

fn draw_session_card(f: &mut Frame, area: Rect, app: &App) {
    let mut t = if let Some(buf) = app.rename.as_ref() {
        format!("rename: {buf}_\n\n")
    } else {
        String::from("enter load  ·  n new  ·  r rename  ·  esc back\n\n")
    };
    if app.session_rows.is_empty() {
        t.push_str("(no chats yet — n starts one)\n");
    } else {
        for (i, row) in app.session_rows.iter().enumerate() {
            let mark = if i == app.session_idx { "▸ " } else { "  " };
            let name = if row.name.is_empty() {
                row.id.as_str()
            } else {
                row.name.as_str()
            };
            let blurb = if row.summary.is_empty() {
                "(empty)"
            } else {
                row.summary.as_str()
            };
            t.push_str(&format!(
                "{mark}{name}  {} you · {}  {blurb}\n",
                row.users,
                mind::age_label(row.mtime)
            ));
        }
    }
    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(t)
            .wrap(Wrap { trim: false })
            .style(Style::default().fg(FG))
            .block(
                Block::default()
                    .title(" sessions  ·  ~/.cache/pixie/sessions  ·  C-s ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(PINK))
                    .border_set(chrome::border_set()),
            ),
        area,
    );
}

fn load_session(app: &mut App, id: &str) {
    let (chat, key) = agent::from_session(id);
    app.chat = chat;
    app.session_id = id.to_string();
    app.session_name = mind::load_meta(id, None).name;
    app.rune_idx = runes::all()
        .iter()
        .position(|r| r.key == key)
        .unwrap_or(app.rune_idx);
    app.scroll = 0;
    app.file_sel = 0;
    app.live.clear();
    app.live_think.clear();
    app.busy = false;
    wake_if_needed(app);
}

fn new_session(app: &mut App) {
    app.chat.clear();
    app.session_id = chrono_id();
    app.session_name = app.session_id.clone();
    app.rune_idx = runes::idx_of(app.default_rune);
    app.scroll = 0;
    app.file_sel = 0;
    app.live.clear();
    app.live_think.clear();
    app.busy = false;
    wake_if_needed(app);
}

fn compact_on_quit(app: &App) {
    let mut lines = Vec::new();
    for e in &app.chat {
        match e {
            ChatEntry::User { text: t, files } if !t.trim().is_empty() || !files.is_empty() => {
                let mut s = t.clone();
                for a in files {
                    if !s.is_empty() {
                        s.push(' ');
                    }
                    s.push_str(&a.path);
                }
                lines.push(("you".into(), s));
            }
            ChatEntry::Assistant { text, .. } if !text.trim().is_empty() => {
                lines.push(("pixie".into(), text.clone()));
            }
            _ => {}
        }
    }
    if lines.is_empty() {
        return;
    }
    let seed = mind::compact_seed(&lines);
    if seed.trim().is_empty() {
        return;
    }
    let msgs = vec![
        json!({
            "role": "system",
            "content": "Summarize this local assistant session into 4 to 8 durable house facts. One markdown bullet per line starting with - . House facts only. No secrets, passwords, keys, client names, NIFs, mail URLs, or Wi-Fi. If nothing durable, reply NONE."
        }),
        json!({"role": "user", "content": seed}),
    ];
    let port = crate::voice::voice_port();
    let Some(raw) = llm::complete_on_if_up(port, &msgs, 160, 3) else {
        return;
    };
    for fact in mind::durable_facts(&raw) {
        let _ = mind::remember(&fact, None);
    }
}

fn draw_plan_card(f: &mut Frame, area: Rect, app: &App) {
    let card = mind::plan_summary(&app.session_id, 80);
    let choices = mind::plan_menu_choices();
    let mut t = if card.is_empty() {
        "(no plan in cache)".into()
    } else {
        card
    };
    t.push_str("\n\n");
    for (i, (k, label, _)) in choices.iter().enumerate() {
        let mark = if i == app.plan_idx { "▸ " } else { "  " };
        t.push_str(&format!("{mark}{k}  {label}\n"));
    }
    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(t)
            .wrap(Wrap { trim: false })
            .style(Style::default().fg(FG))
            .block(
                Block::default()
                    .title(" plan  ·  cache only (~/.cache/pixie/plans)  ·  arrows+enter ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(PINK))
                    .border_set(chrome::border_set()),
            ),
        area,
    );
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
        f.render_widget(Paragraph::new(*line).style(Style::default().fg(PINK)), row);
    }
    let wait = if let Some(err) = app.wake_err.as_deref() {
        if err.len() > 48 {
            "menagerie did not wake — retrying"
        } else {
            err
        }
    } else {
        glyph::wait_line(app.born)
    };
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

fn draw_look(f: &mut Frame, area: Rect, app: &mut App) {
    let Some((path, image, look_lines, scroll)) = app.look.as_ref().map(|look| {
        (
            look.path.clone(),
            look.image,
            look.lines.clone(),
            look.scroll,
        )
    }) else {
        return;
    };
    let title = format!(" look · {} ", pretty_path(&path));
    let inner_w = area.width.saturating_sub(2).max(8) as usize;
    let mut lines: Vec<Line> = Vec::new();
    if image && Path::new(&path).is_file() {
        let max_cols = area.width.saturating_sub(4).clamp(8, 72);
        let max_rows = area.height.saturating_sub(4).clamp(4, 24);
        if kitty::is_terminal() {
            let place = kitty::fit_path(Path::new(&path), max_cols, max_rows);
            lines.extend(kitty::placeholder_lines(place.id, place.cols, place.rows));
            app.kitty_frame.insert(place.id, place);
        } else {
            lines.extend(crate::forge::png_halfblocks(
                Path::new(&path),
                max_cols,
                max_rows,
            ));
        }
        for (op, text) in &look_lines {
            lines.push(diff_line(*op, text));
        }
    } else {
        for (op, text) in &look_lines {
            for chunk in chrome::wrap(text, inner_w.saturating_sub(2)) {
                lines.push(diff_line(*op, &chunk));
            }
        }
    }
    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "(empty)",
            Style::default().fg(DIM),
        )));
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
        .scroll((scroll, 0));
    f.render_widget(para, area);
}

fn draw_chat(f: &mut Frame, area: Rect, app: &mut App, rune_name: &str) {
    let pct = agent::thread_pct(&app.chat);
    let meter = chrome::thread_meter(pct);
    let title = format!(" ✦ Pixie ✦ {rune_name} · {}  {meter} ", app.session_name);
    let border = if pct >= 70 { PINK } else { BLUSH };
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border))
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
        let max_w =
            ((inner.width as u32 * b.max_frac as u32) / 100).clamp(12, inner.width as u32) as u16;
        let body = bubble_body_text(b);
        let w = chrome::bubble_width(&body, max_w, 12).min(inner.width);
        let inner_w = w.saturating_sub(2).max(1) as usize;
        let h = 2u16 + wrapped_height(&b.lines, inner_w);
        heights.push((w, h.max(3)));
    }

    let gaps = bubbles.len().saturating_sub(1) as u16;
    let total: u16 = heights
        .iter()
        .map(|(_, h)| *h)
        .sum::<u16>()
        .saturating_add(gaps);
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
        let Some((vis_y, vis_h, skip)) = bubble_clip(y, h, inner.y as i32, bottom) else {
            continue;
        };
        let r = Rect {
            x,
            y: vis_y,
            width: w,
            height: vis_h,
        };
        render_bubble(f, r, b, app.file_sel, skip);
        if let Some(fi) = b.file_i {
            app.file_hits.push((r, fi));
        }
        if skip == 0 {
            if let Some(place) = &b.place {
                app.kitty_frame.insert(place.id, place.clone());
            }
        }
    }
}

fn collect_bubbles(app: &App, _width: u16) -> Vec<Bubble> {
    let mut out = Vec::new();
    let mut file_i = 0usize;
    let mut think_n = 0usize;
    let mut tool_n = 0usize;
    for e in &app.chat {
        match e {
            ChatEntry::User { text: t, files } => {
                if !t.trim().is_empty() {
                    out.push(Bubble {
                        right: true,
                        title: " you ".into(),
                        lines: wrap_plain(t),
                        max_frac: 70,
                        border: BLUSH,
                        file_i: None,
                        place: None,
                    });
                }
                for a in files {
                    let sel = file_i == app.file_sel;
                    out.push(clip_bubble(a, sel, file_i));
                    file_i += 1;
                }
            }
            ChatEntry::Assistant {
                text,
                clock,
                think,
                cancelled,
            } => {
                push_think_bubble(&mut out, app.verbose, think, &mut think_n);
                let mark = if *cancelled { " (cancelled)" } else { "" };
                out.push(Bubble {
                    right: false,
                    title: format!(" ✦ Pixie · {clock}{mark} "),
                    lines: wrap_plain(text),
                    max_frac: 80,
                    border: PINK,
                    file_i: None,
                    place: None,
                });
            }
            ChatEntry::Compact { .. } => {}
            ChatEntry::Tool {
                name,
                title,
                body,
                file,
                think,
                ok,
            } => {
                push_think_bubble(&mut out, app.verbose, think, &mut think_n);
                tool_n += 1;
                let label = tools::tool_label(name);
                let border = if *ok { GREEN } else { RED };
                if let Some(ft) = file {
                    let sel = file_i == app.file_sel;
                    out.push(file_bubble(
                        ft,
                        sel,
                        app.verbose,
                        body,
                        file_i,
                        tool_n,
                        name,
                        *ok,
                    ));
                    file_i += 1;
                } else {
                    out.push(Bubble {
                        right: false,
                        title: if app.verbose {
                            format!(" tool {tool_n} · {title} ")
                        } else {
                            format!(" {label} ")
                        },
                        lines: wrap_plain(body),
                        max_frac: 90,
                        border,
                        file_i: None,
                        place: None,
                    });
                }
            }
        }
    }
    if app.busy {
        push_think_bubble(&mut out, app.verbose, &app.live_think, &mut think_n);
        let vis = live_speech(app);
        let spin = glyph::spin_line(app.born);
        if !vis.is_empty() {
            out.push(Bubble {
                right: false,
                title: format!(" ✦ Pixie · {spin} "),
                lines: wrap_plain(&vis),
                max_frac: 80,
                border: PINK,
                file_i: None,
                place: None,
            });
        } else {
            out.push(Bubble {
                right: false,
                title: format!(" ✦ Pixie · {spin} "),
                lines: wrap_plain(glyph::wait_line(app.born)),
                max_frac: 50,
                border: SILVER,
                file_i: None,
                place: None,
            });
        }
    }
    out
}

fn push_think_bubble(out: &mut Vec<Bubble>, verbose: bool, think: &str, think_n: &mut usize) {
    if !verbose || think.trim().is_empty() {
        return;
    }
    *think_n += 1;
    out.push(Bubble {
        right: false,
        title: format!(" think {think_n} "),
        lines: wrap_plain(think),
        max_frac: 80,
        border: DIM,
        file_i: None,
        place: None,
    });
}

#[cfg(test)]
fn entry_titles(chat: &[ChatEntry], verbose: bool) -> Vec<String> {
    let mut titles = Vec::new();
    let mut think_n = 0usize;
    let mut tool_n = 0usize;
    for e in chat {
        match e {
            ChatEntry::User { .. } => titles.push("you".into()),
            ChatEntry::Compact { .. } => {}
            ChatEntry::Assistant { think, .. } => {
                if verbose && !think.trim().is_empty() {
                    think_n += 1;
                    titles.push(format!("think {think_n}"));
                }
                titles.push("pixie".into());
            }
            ChatEntry::Tool { name, think, .. } => {
                if verbose && !think.trim().is_empty() {
                    think_n += 1;
                    titles.push(format!("think {think_n}"));
                }
                tool_n += 1;
                titles.push(format!("tool {tool_n} · {}", tools::tool_label(name)));
            }
        }
    }
    titles
}

fn file_bubble(
    ft: &FileTouch,
    sel: bool,
    verbose: bool,
    body: &str,
    file_i: usize,
    tool_n: usize,
    name: &str,
    ok: bool,
) -> Bubble {
    let sum = ft.summary();
    let mark = if sel { "▸ " } else { "" };
    let label = tools::tool_label(name);
    let tool = if name.is_empty() {
        format!("tool {tool_n}")
    } else {
        format!("tool {tool_n} · {label}")
    };
    let title = if sum.is_empty() {
        format!(
            " {mark}{tool} · {} · {} ",
            ft.kind.as_str(),
            pretty_path(&ft.path)
        )
    } else {
        format!(
            " {mark}{tool} · {} · {} · {sum} ",
            ft.kind.as_str(),
            pretty_path(&ft.path)
        )
    };
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut place = None;
    if ft.kind == diff::FileKind::Image {
        let (img_lines, p) = picture_lines(Path::new(&ft.path), 72, 24);
        lines.extend(img_lines);
        place = p;
        if !body.is_empty() {
            lines.push(Line::from(Span::styled(
                body.to_string(),
                Style::default().fg(SILVER),
            )));
        }
    } else if ft.kind == diff::FileKind::Search && !ft.extra_paths.is_empty() {
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
        "C-o open  ·  C-l look  ·  C-p / C-n files",
        Style::default().fg(DIM),
    )));
    if verbose && !body.is_empty() && ft.kind != diff::FileKind::Image {
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
        border: if sel {
            PINK
        } else if ok {
            GREEN
        } else {
            RED
        },
        file_i: Some(file_i),
        place,
    }
}

fn picture_lines(path: &Path, max_cols: u16, max_rows: u16) -> (Vec<Line<'static>>, Option<kitty::Place>) {
    if !path.is_file() {
        return (
            vec![Line::from(Span::styled(
                "picture gone",
                Style::default().fg(DIM),
            ))],
            None,
        );
    }
    if kitty::is_terminal() {
        let place = kitty::fit_path(path, max_cols, max_rows);
        let lines = kitty::placeholder_lines(place.id, place.cols, place.rows);
        (lines, Some(place))
    } else {
        (
            crate::forge::png_halfblocks(path, max_cols, max_rows.max(8)),
            None,
        )
    }
}

fn clip_bubble(a: &Attach, sel: bool, file_i: usize) -> Bubble {
    let mark = if sel { "▸ " } else { "" };
    let name = pretty_path(&a.path);
    match a.kind {
        attach::Kind::Image => {
            let (mut lines, place) = picture_lines(Path::new(&a.path), 56, 20);
            if let Some(c) = &a.caption {
                lines.push(Line::from(Span::styled(
                    c.clone(),
                    Style::default().fg(SILVER),
                )));
            }
            Bubble {
                right: true,
                title: format!(" {mark}you · picture · {name} "),
                lines,
                max_frac: 70,
                border: if sel { PINK } else { BLUSH },
                file_i: Some(file_i),
                place,
            }
        }
        attach::Kind::Text => Bubble {
            right: true,
            title: format!(" {mark}you · text · {name} "),
            lines: wrap_plain(
                &a.excerpt(400)
                    .unwrap_or_else(|| "(empty)".into()),
            ),
            max_frac: 70,
            border: if sel { PINK } else { BLUSH },
            file_i: Some(file_i),
            place: None,
        },
        attach::Kind::Other => Bubble {
            right: true,
            title: format!(" {mark}you · file · {name} "),
            lines: wrap_plain("pixie cannot see this kind yet"),
            max_frac: 50,
            border: if sel { PINK } else { DIM },
            file_i: Some(file_i),
            place: None,
        },
    }
}

fn diff_line(op: Op, text: &str) -> Line<'static> {
    match op {
        Op::Add => Line::from(vec![
            Span::styled(
                "+ ",
                Style::default().fg(GREEN).add_modifier(Modifier::BOLD),
            ),
            Span::styled(text.to_string(), Style::default().fg(GREEN)),
        ]),
        Op::Del => Line::from(vec![
            Span::styled("- ", Style::default().fg(RED).add_modifier(Modifier::BOLD)),
            Span::styled(text.to_string(), Style::default().fg(RED)),
        ]),
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
    text.lines().map(|l| Line::from(l.to_string())).collect()
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
        if t.contains(kitty::PLACEHOLDER) {
            n = n.saturating_add(1);
            continue;
        }
        let chunks = chrome::wrap(&t, inner_w.max(1));
        n = n.saturating_add(chunks.len().max(1) as u16);
    }
    n.max(1)
}

fn render_bubble(f: &mut Frame, rect: Rect, b: &Bubble, file_sel: usize, skip: u16) {
    let border = if b.file_i == Some(file_sel) && b.file_i.is_some() {
        PINK
    } else {
        b.border
    };
    let wrap = Wrap {
        trim: false,
    };
    let para = Paragraph::new(b.lines.clone())
        .block(
            Block::default()
                .title(b.title.clone())
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border))
                .border_set(chrome::border_set()),
        )
        .wrap(wrap)
        .scroll((skip, 0));
    f.render_widget(para, rect);
}

/// Visible slice of a bubble that may start above the chat viewport.
/// Old path skipped `y < inner.y`, so a last bubble taller than the view
/// painted nothing — Pixie replies vanished when verbose think/tools were huge.
fn bubble_clip(y: i32, h: u16, inner_y: i32, bottom: i32) -> Option<(u16, u16, u16)> {
    let y_end = y + h as i32;
    if y_end <= inner_y || y >= bottom {
        return None;
    }
    let vis_y = y.max(inner_y);
    let vis_h = y_end.min(bottom) - vis_y;
    if vis_h < 2 {
        return None;
    }
    Some((vis_y as u16, vis_h as u16, (vis_y - y) as u16))
}

#[cfg(test)]
mod tests {
    use super::{bubble_clip, entry_titles, mask_live_speech, sanitize_input};
    use crate::agent::ChatEntry;

    #[test]
    fn sanitize_keeps_normal_typing() {
        assert_eq!(sanitize_input("ok now"), "ok now");
        assert_eq!(sanitize_input("? help"), "? help");
    }

    #[test]
    fn sanitize_drops_sgr_mouse() {
        assert_eq!(sanitize_input("\x1b[<35;78;49M"), "");
        assert_eq!(sanitize_input("[<35;78;49M"), "");
        assert_eq!(sanitize_input("<35;78;49M"), "");
        assert_eq!(sanitize_input("hello[<0;10;10Mworld"), "helloworld");
    }

    #[test]
    fn file_card_teaches_ctrl_keys() {
        let src = include_str!("tui.rs");
        assert!(
            src.contains("C-o open  ·  C-l look  ·  C-p / C-n files"),
            "file card must teach Ctrl keys"
        );
        assert!(src.contains("C-u clip"), "help must teach C-u clip");
        assert!(
            src.contains("  · C-v verbose"),
            "footer must label C-v as verbose"
        );
        assert!(
            src.contains("A-v speak"),
            "help must teach Alt-v as speak"
        );
        assert!(
            src.contains("KeyModifiers::ALT | KeyModifiers::SUPER"),
            "speak toggle is Alt/Super+v"
        );
        assert!(src.contains("C-s sessions"), "help must teach C-s sessions");
        assert!(src.contains("n new"), "dashboard must offer New");
        assert!(src.contains("Options"), "dashboard must offer Options");
        assert!(src.contains("r rename"), "session list must teach r rename");
        assert!(src.contains("d delete"), "dashboard must teach d delete");
        assert!(src.contains("esc home"), "help must teach esc home");
        assert!(src.contains("thread_meter"), "chat title uses thread_meter");
    }

    #[test]
    fn tall_last_bubble_stays_visible() {
        // view rows 1..21 (height 20). Last bubble height 40 sits at y = 1+20-40 = -19.
        let (vis_y, vis_h, skip) = bubble_clip(-19, 40, 1, 21).expect("must paint");
        assert_eq!(vis_y, 1);
        assert_eq!(vis_h, 20);
        assert_eq!(skip, 20);
    }

    #[test]
    fn fully_above_bubble_is_skipped() {
        assert!(bubble_clip(-10, 5, 1, 21).is_none());
    }

    #[test]
    fn think_and_tool_are_enumerated_both() {
        let chat = vec![
            ChatEntry::you("hi"),
            ChatEntry::Tool {
                name: "execute_bash".into(),
                title: "execute_bash · 0.2s".into(),
                body: "ok".into(),
                file: None,
                think: "I should run ls".into(),
                ok: true,
            },
            ChatEntry::Assistant {
                text: "done".into(),
                clock: "12:00:00".into(),
                think: "now answer".into(),
                cancelled: false,
            },
        ];
        assert_eq!(
            entry_titles(&chat, true),
            vec![
                "you".to_string(),
                "think 1".into(),
                "tool 1 · shell".into(),
                "think 2".into(),
                "pixie".into(),
            ]
        );
        assert_eq!(
            entry_titles(&chat, false),
            vec![
                "you".to_string(),
                "tool 1 · shell".into(),
                "pixie".into(),
            ]
        );
        let mut with_compact = chat.clone();
        with_compact.insert(
            1,
            ChatEntry::Compact {
                text: "earlier recap".into(),
            },
        );
        assert_eq!(
            entry_titles(&with_compact, false),
            vec![
                "you".to_string(),
                "tool 1 · shell".into(),
                "pixie".into(),
            ]
        );
    }

    #[test]
    fn mask_hides_json_tool_only() {
        let json = r#"{"name": "execute_bash", "arguments": {"command": "ls"}}"#;
        assert_eq!(mask_live_speech(json, None), "calling execute_bash…");
        assert_eq!(
            mask_live_speech("Sparkle magic! Ready when you are.", None),
            "Sparkle magic! Ready when you are."
        );
        let prose = "I already ran execute_bash. Here is the answer for you.";
        assert_eq!(mask_live_speech(prose, None), prose);
        assert_eq!(
            mask_live_speech("anything", Some("execute_bash")),
            "calling execute_bash…"
        );
    }
}
