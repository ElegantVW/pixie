mod agent;
mod attach;
mod chrome;
mod diff;
mod forge;
mod glyph;
mod grove_keys;
mod host;
mod kitty;
mod llm;
mod mind;
mod paths;
mod prefs;
mod privilege;
mod runes;
mod speak;
mod think;
mod tools;
mod tui;
mod voice;
mod wa_auto;

use crate::agent::ChatEntry;
use crate::llm::Abort;
use crate::mind::SessionRec;
use crate::runes::RuneKey;
use anyhow::Result;
use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "pixie", version, about = "local faeOS familiar")]
struct Cli {
    /// Show think + full tool mirrors
    #[arg(short, long)]
    verbose: bool,
    /// List chat archives
    #[arg(long)]
    chats: bool,
    /// Print a chat transcript (latest if omitted)
    #[arg(long, num_args = 0..=1, default_missing_value = "")]
    chat: Option<String>,
    /// Resume a chat in the TUI (latest if omitted)
    #[arg(long, num_args = 0..=1, default_missing_value = "")]
    resume: Option<String>,
    /// Start a fresh TUI session instead of resuming the latest
    #[arg(short = 'n', long = "new", conflicts_with = "resume")]
    new: bool,
    /// sudo askpass helper (stdout only). Hidden.
    #[arg(long, hide = true)]
    askpass: bool,
    /// Headless WhatsApp auto secretary (user unit pixie-auto.service)
    #[arg(long)]
    auto: bool,
    /// Auto classify fixtures. No follow, no send, no model.
    #[arg(long)]
    self_test: bool,
    /// One-shot prompt
    prompt: Vec<String>,
}

fn main() {
    if std::env::args().any(|a| a == "--askpass") {
        std::process::exit(crate::privilege::askpass_client());
    }
    let code = match real_main() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("pixie: {e}");
            eprintln!("  next:  check the menagerie is awake (`menagerie ensure pixie`)");
            1
        }
    };
    std::process::exit(code);
}

fn real_main() -> Result<i32> {
    let cli = Cli::parse();
    if cli.askpass {
        return Ok(crate::privilege::askpass_client());
    }
    if cli.self_test {
        return crate::wa_auto::self_test();
    }
    if cli.auto {
        return crate::wa_auto::run();
    }
    if cli.chats {
        println!("{}", mind::format_chat_list(None));
        return Ok(0);
    }
    if let Some(id) = cli.chat {
        let name = if id.is_empty() {
            None
        } else {
            Some(id.as_str())
        };
        let Some(sid) = mind::resolve_session_id(name, None) else {
            if id.is_empty() {
                eprintln!("pixie: no chats stored yet");
            } else {
                eprintln!("pixie: no chat named {id}");
            }
            return Ok(1);
        };
        println!("{}", mind::format_chat(&sid, None));
        return Ok(0);
    }
    if let Some(id) = cli.resume {
        let name = if id.is_empty() {
            None
        } else {
            Some(id.as_str())
        };
        let Some(sid) = mind::resolve_session_id(name, None) else {
            if id.is_empty() {
                eprintln!("pixie: no chats stored yet");
            } else {
                eprintln!("pixie: no chat named {id}");
            }
            return Ok(1);
        };
        tui::run(cli.verbose, Some(sid), false)?;
        return Ok(0);
    }
    if cli.prompt.is_empty() {
        tui::run(cli.verbose, None, mind::wants_door(cli.new, false))?;
        return Ok(0);
    }
    oneshot(&cli.prompt.join(" "), cli.verbose)
}

fn oneshot(prompt: &str, verbose: bool) -> Result<i32> {
    mind::seed_memory(None)?;
    if !llm::llm_up() && !llm::ensure_llm_up() {
        eprintln!("pixie: the menagerie brain is asleep");
        eprintln!("  next:  menagerie ensure pixie");
        return Ok(1);
    }
    let rune = runes::by_key(RuneKey::Chat);
    let sid = format!("oneshot-{}", std::process::id());
    mind::append_session(
        &sid,
        &SessionRec {
            ts: mind::now_ts(),
            role: "user".into(),
            rune: "oneshot".into(),
            text: prompt.into(),
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
    let mut chat = vec![ChatEntry::you(prompt)];
    let abort = Abort::new();
    let t0 = std::time::Instant::now();
    let out = agent::run_turn(&mut chat, &rune, &sid, &abort, verbose, None, |_| {});
    let text = if let Some(e) = out.err {
        format!("(✦ pixie lost the connection: {e})")
    } else if out.cancelled {
        out.text
    } else if crate::prefs::load_ui()?.sparkle {
        crate::voice::prepend("final answer for the human", &out.text)
    } else {
        out.text
    };
    mind::append_session(
        &sid,
        &SessionRec {
            ts: mind::now_ts(),
            role: "assistant".into(),
            rune: "oneshot".into(),
            text: text.clone(),
            tool: String::new(),
            args: None,
            result: None,
            raw: String::new(),
            think: out.think.clone(),
            cancelled: out.cancelled,
            from_rune: String::new(),
            to_rune: String::new(),
            reason: String::new(),
        },
        None,
    );
    let elapsed = t0.elapsed().as_secs_f32();
    let mut body = text;
    if verbose && !out.think.is_empty() {
        body = format!("think: {}\n\n{body}", out.think);
    }
    println!(
        "{}",
        chrome::box_text(
            &format!("Pixie ─────── ✦ Thought for {elapsed:.2}s"),
            &body,
            chrome::term_width()
        )
    );
    Ok(0)
}
