//! WhatsApp away secretary. Options Auto owns a user unit; the TUI may quit.
//! Chat `check_whatsapp` still never `sync --follow`.

use crate::agent::{self, ChatEntry};
use crate::host;
use crate::llm::{self, Abort};
use crate::mind::{self, SessionRec};
use crate::paths;
use crate::prefs;
use crate::speak::{self, Tongue};
use crate::think;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

pub const FALLBACK: &str =
    "Hi, this is Pixie. Gil cannot reach WhatsApp right now. Can I help with anything?";

const COALESCE: Duration = Duration::from_secs(8);
const POLL: Duration = Duration::from_secs(4);
const SEEN_CAP: usize = 500;
const UNIT_NAME: &str = "pixie-auto.service";
const UNIT_BODY: &str = include_str!("../systemd/pixie-auto.service");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InboundKind {
    Text,
    VoiceNote,
    Media,
    CallMissed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplyKind {
    Text,
    Voice,
}

#[derive(Clone, Debug)]
pub struct Inbound {
    pub id: String,
    pub chat: String,
    pub kind: InboundKind,
    pub text: String,
    pub media_kind: String,
    pub caption: String,
    pub reply_to: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Burst {
    pub chat: String,
    pub items: Vec<Inbound>,
}

struct Pending {
    burst: Burst,
    due: Instant,
}

struct State {
    since: String,
    seen: Vec<String>,
}

pub fn classify_message(
    from_me: bool,
    chat_jid: &str,
    media_type: &str,
    mime: &str,
    reaction_to: &str,
    revoked: bool,
    deleted: bool,
) -> Option<InboundKind> {
    if from_me || revoked || deleted {
        return None;
    }
    if is_group_jid(chat_jid) {
        return None;
    }
    if !reaction_to.trim().is_empty() {
        return None;
    }
    let mt = media_type.trim().to_ascii_lowercase();
    let mime = mime.trim().to_ascii_lowercase();
    if mt.is_empty() {
        return Some(InboundKind::Text);
    }
    if mt == "audio" || mt == "ptt" || mime.contains("ogg") || mime.contains("opus") {
        return Some(InboundKind::VoiceNote);
    }
    Some(InboundKind::Media)
}

pub fn classify_call(direction: &str, outcome: &str) -> Option<InboundKind> {
    if direction.trim().eq_ignore_ascii_case("outbound") {
        return None;
    }
    if !direction.trim().eq_ignore_ascii_case("inbound") {
        return None;
    }
    if outcome.trim().eq_ignore_ascii_case("connected") {
        return None;
    }
    Some(InboundKind::CallMissed)
}

pub fn is_group_jid(jid: &str) -> bool {
    let j = jid.to_ascii_lowercase();
    j.contains("@g.us") || j.contains("@broadcast") || j.contains("@newsletter")
}

pub fn reply_kind(items: &[Inbound]) -> ReplyKind {
    if items
        .iter()
        .any(|i| matches!(i.kind, InboundKind::VoiceNote | InboundKind::CallMissed))
    {
        ReplyKind::Voice
    } else {
        ReplyKind::Text
    }
}

pub fn merge_burst(mut cur: Burst, next: Inbound) -> Burst {
    if cur.chat.is_empty() {
        cur.chat = next.chat.clone();
    }
    cur.items.push(next);
    cur
}

/// Drop hallucinated tool JSON / fences. Empty leftover becomes FALLBACK.
pub fn spoken_reply(raw: &str) -> String {
    let vis = think::strip(raw);
    let stripped = strip_tool_spans(&vis);
    let cleaned = strip_fences(&stripped);
    let t = cleaned.trim();
    if t.is_empty() || looks_like_tool(t) {
        FALLBACK.to_string()
    } else {
        t.to_string()
    }
}

fn strip_tool_spans(text: &str) -> String {
    let mut calls = mind::extract_tool_calls(text, 8);
    if calls.is_empty() {
        return text.to_string();
    }
    calls.sort_by_key(|c| c.start);
    let mut out = String::new();
    let mut at = 0usize;
    for c in calls {
        if c.start >= at && c.end <= text.len() && c.end > c.start {
            out.push_str(&text[at..c.start]);
            at = c.end;
        }
    }
    out.push_str(&text[at..]);
    out
}

fn strip_fences(text: &str) -> String {
    let mut s = text.to_string();
    while let Some(i) = s.find("```") {
        if let Some(j) = s[i + 3..].find("```") {
            let end = i + 3 + j + 3;
            s.replace_range(i..end, " ");
        } else {
            s.replace_range(i.., " ");
            break;
        }
    }
    s
}

fn looks_like_tool(text: &str) -> bool {
    let t = text.trim();
    if t.starts_with('{') && t.contains("\"name\"") && t.contains("\"arguments\"") {
        return true;
    }
    t.contains("<tool_call>")
}

pub fn chat_key(jid: &str) -> String {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    jid.hash(&mut h);
    format!("wa-auto-{:016x}", h.finish())
}

pub fn apply_enabled(on: bool) -> Result<(), String> {
    if cfg!(test) {
        return Ok(());
    }
    install_unit()?;
    let action = if on {
        ["enable", "--now"]
    } else {
        ["disable", "--now"]
    };
    systemctl(&[&action[0], action[1], UNIT_NAME])
}

pub fn unit_active() -> bool {
    if cfg!(test) {
        return false;
    }
    Command::new("systemctl")
        .args(["--user", "is-active", "--quiet", UNIT_NAME])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn install_unit() -> Result<(), String> {
    let engine = paths::home().join(".local/lib/faeos/pixie");
    if !engine.is_file() {
        return Err("auto needs a house-install (./build.sh install --house)".into());
    }
    let dir = paths::home().join(".config/systemd/user");
    fs::create_dir_all(&dir).map_err(|e| format!("pixie: auto unit dir: {e}"))?;
    let dest = dir.join(UNIT_NAME);
    fs::write(&dest, UNIT_BODY).map_err(|e| format!("pixie: auto unit: {e}"))?;
    systemctl(&["daemon-reload"])
}

fn systemctl(args: &[&str]) -> Result<(), String> {
    let mut cmd = Command::new("systemctl");
    cmd.arg("--user");
    cmd.args(args);
    let out = cmd
        .output()
        .map_err(|e| format!("pixie: systemctl: {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    let clip: String = err.chars().take(180).collect();
    Err(if clip.trim().is_empty() {
        format!("systemctl {} failed", args.join(" "))
    } else {
        clip.trim().to_string()
    })
}

/// Headless loop. systemd `pixie --auto`. TUI quit leaves this running.
pub fn run() -> anyhow::Result<i32> {
    if !prefs::auto_on() {
        eprintln!("pixie: auto is off");
        return Ok(0);
    }
    mind::seed_memory(None)?;
    if !llm::llm_up() && !llm::ensure_llm_up() {
        eprintln!("pixie: the menagerie brain is asleep");
        eprintln!("  next:  menagerie ensure pixie");
        return Ok(1);
    }
    let mut w = Watcher::start()?;
    while prefs::auto_on() {
        if let Err(e) = w.tick() {
            log_line(&format!("tick: {e}"));
        }
        std::thread::sleep(POLL);
    }
    w.stop();
    Ok(0)
}

struct Watcher {
    follow: Option<Child>,
    pending: HashMap<String, Pending>,
    state: State,
}

impl Watcher {
    fn start() -> anyhow::Result<Self> {
        let mut state = load_state();
        if state.since.is_empty() {
            state.since = rfc3339_now();
            save_state(&state);
        }
        let follow = spawn_follow();
        if follow.is_none() {
            log_line("follow did not start (store busy?)");
        }
        Ok(Self {
            follow,
            pending: HashMap::new(),
            state,
        })
    }

    fn stop(&mut self) {
        kill_child(&mut self.follow);
    }

    fn tick(&mut self) -> Result<(), String> {
        if !child_alive(&mut self.follow) {
            self.follow = spawn_follow();
        }
        let msgs = list_new_messages(&self.state.since)?;
        let calls = list_new_calls(&self.state.since)?;
        let mut newest = self.state.since.clone();
        for m in msgs {
            if let Some(ts) = m.get("Timestamp").and_then(|x| x.as_str()) {
                if ts > newest.as_str() {
                    newest = ts.to_string();
                }
            }
            if let Some(inb) = inbound_from_msg(&m) {
                self.note(inb);
            }
        }
        for c in calls {
            if let Some(ts) = c.get("timestamp").and_then(|x| x.as_str()) {
                if ts > newest.as_str() {
                    newest = ts.to_string();
                }
            }
            if let Some(inb) = inbound_from_call(&c) {
                self.note(inb);
            }
        }
        if newest > self.state.since {
            self.state.since = newest;
            save_state(&self.state);
        }
        self.flush_due();
        Ok(())
    }

    fn note(&mut self, inb: Inbound) {
        if self.state.seen.iter().any(|s| s == &inb.id) {
            return;
        }
        remember(&mut self.state, &inb.id);
        save_state(&self.state);
        let chat = inb.chat.clone();
        match self.pending.get_mut(&chat) {
            Some(p) => {
                p.burst = merge_burst(p.burst.clone(), inb);
            }
            None => {
                self.pending.insert(
                    chat.clone(),
                    Pending {
                        burst: Burst {
                            chat,
                            items: vec![inb],
                        },
                        due: Instant::now() + COALESCE,
                    },
                );
            }
        }
    }

    fn flush_due(&mut self) {
        let now = Instant::now();
        let ready: Vec<String> = self
            .pending
            .iter()
            .filter(|(_, p)| p.due <= now)
            .map(|(k, _)| k.clone())
            .collect();
        for k in ready {
            if let Some(p) = self.pending.remove(&k) {
                if let Err(e) = handle_burst(&p.burst) {
                    log_line(&format!("send: {e}"));
                }
            }
        }
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.stop();
    }
}

fn inbound_from_msg(m: &Value) -> Option<Inbound> {
    let from_me = m.get("FromMe").and_then(|x| x.as_bool()).unwrap_or(false);
    let chat = m
        .get("ChatJID")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let media = m.get("MediaType").and_then(|x| x.as_str()).unwrap_or("");
    let mime = m.get("MimeType").and_then(|x| x.as_str()).unwrap_or("");
    let reaction = m.get("ReactionToID").and_then(|x| x.as_str()).unwrap_or("");
    let revoked = m.get("Revoked").and_then(|x| x.as_bool()).unwrap_or(false);
    let deleted = m
        .get("DeletedForMe")
        .and_then(|x| x.as_bool())
        .unwrap_or(false);
    let kind = classify_message(from_me, &chat, media, mime, reaction, revoked, deleted)?;
    let id = m
        .get("MsgID")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    if id.is_empty() || chat.is_empty() {
        return None;
    }
    let text = m
        .get("Text")
        .or_else(|| m.get("DisplayText"))
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let caption = m
        .get("MediaCaption")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    Some(Inbound {
        id: id.clone(),
        chat,
        kind,
        text,
        media_kind: media.to_string(),
        caption,
        reply_to: Some(id),
    })
}

fn inbound_from_call(c: &Value) -> Option<Inbound> {
    let direction = c.get("direction").and_then(|x| x.as_str()).unwrap_or("");
    let outcome = c.get("outcome").and_then(|x| x.as_str()).unwrap_or("");
    let kind = classify_call(direction, outcome)?;
    let id = c
        .get("call_id")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let chat = c
        .get("chat_jid")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    if id.is_empty() || chat.is_empty() || is_group_jid(&chat) {
        return None;
    }
    Some(Inbound {
        id,
        chat,
        kind,
        text: String::new(),
        media_kind: c
            .get("media")
            .and_then(|x| x.as_str())
            .unwrap_or("audio")
            .to_string(),
        caption: String::new(),
        reply_to: None,
    })
}

fn handle_burst(burst: &Burst) -> Result<(), String> {
    if cfg!(test) {
        return Ok(());
    }
    if burst.items.is_empty() {
        return Ok(());
    }
    let kind = reply_kind(&burst.items);
    let sid = chat_key(&burst.chat);
    let (mut chat, _) = agent::from_session(&sid);
    let prompt = inbound_prompt(burst, chat.is_empty());
    chat.push(ChatEntry::you(&prompt));
    append_user(&sid, &prompt);
    let abort = Abort::new();
    let out = run_auto_turn(&chat, &abort);
    let raw = if let Some(e) = out.err {
        log_line(&format!("llm: {e}"));
        FALLBACK.to_string()
    } else {
        spoken_reply(&out.text)
    };
    append_assistant(&sid, &raw, &out.think);
    let reply_to = burst
        .items
        .iter()
        .rev()
        .find_map(|i| i.reply_to.clone());
    match kind {
        ReplyKind::Text => send_text(&burst.chat, &raw, reply_to.as_deref()),
        ReplyKind::Voice => send_voice(&burst.chat, &raw, reply_to.as_deref()),
    }
}

fn inbound_prompt(burst: &Burst, first_in_chat: bool) -> String {
    let mut s = String::from("WhatsApp inbound (no names).\n");
    if first_in_chat {
        s.push_str("First ping in this chat while Auto is on.\n");
    }
    for (i, it) in burst.items.iter().enumerate() {
        s.push_str(&format!("\n[{}]\n", i + 1));
        match it.kind {
            InboundKind::Text => {
                s.push_str("Kind: text.\n");
                if !it.text.trim().is_empty() {
                    s.push_str("They wrote:\n");
                    s.push_str(it.text.trim());
                    s.push('\n');
                }
            }
            InboundKind::VoiceNote => {
                s.push_str("Kind: voice note. Reply as a short spoken voice note.\n");
                if let Some(tr) = transcribe_inbound(it) {
                    s.push_str("Transcript:\n");
                    s.push_str(&tr);
                    s.push('\n');
                } else if !it.text.trim().is_empty() {
                    s.push_str("Caption:\n");
                    s.push_str(it.text.trim());
                    s.push('\n');
                } else {
                    s.push_str("No transcript on disk yet.\n");
                }
            }
            InboundKind::Media => {
                let k = if it.media_kind.trim().is_empty() {
                    "file"
                } else {
                    it.media_kind.trim()
                };
                s.push_str(&format!(
                    "Kind: {k}. You cannot view files or pictures yet. Say that briefly, then continue the away speech.\n"
                ));
                if !it.caption.trim().is_empty() {
                    s.push_str("Caption:\n");
                    s.push_str(it.caption.trim());
                    s.push('\n');
                }
            }
            InboundKind::CallMissed => {
                s.push_str(
                    "Kind: missed or unanswered inbound call. Reply as a short spoken voice note.\n",
                );
            }
        }
    }
    s.push_str("\nWrite only the words to send. No JSON. No tools.");
    s
}

fn transcribe_inbound(it: &Inbound) -> Option<String> {
    if !matches!(it.kind, InboundKind::VoiceNote) {
        return None;
    }
    let model = whisper_model()?;
    let dest = cache_dir().join(format!("{}.ogg", safe_id(&it.id)));
    if download_media(&it.id, &it.chat, &dest).is_err() {
        return None;
    }
    let text = transcribe(&dest, &model);
    let _ = fs::remove_file(&dest);
    text.ok().filter(|t| !t.trim().is_empty())
}

fn whisper_model() -> Option<PathBuf> {
    let dir = paths::data_dir().join("models");
    for name in [
        "ggml-small.bin",
        "ggml-base.bin",
        "ggml-tiny.bin",
        "ggml-small.en.bin",
    ] {
        let p = dir.join(name);
        if file_min(&p, 10_000_000) {
            return Some(p);
        }
    }
    None
}

fn transcribe(audio: &Path, model: &Path) -> Result<String, String> {
    let bin = crate::speak::cpu_bin();
    if !file_min(&bin, 1_000_000) {
        return Err("no crispasr".into());
    }
    let out = Command::new(&bin)
        .args([
            "--backend",
            "whisper",
            "--no-gpu",
            "-m",
            &model.display().to_string(),
            "-f",
            &audio.display().to_string(),
            "-t",
            "8",
            "--no-prints",
            "--no-timestamps",
        ])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err("whisper failed".into());
    }
    let t = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if t.is_empty() {
        Err("empty transcript".into())
    } else {
        Ok(t.chars().take(800).collect())
    }
}

fn download_media(id: &str, chat: &str, dest: &Path) -> Result<(), String> {
    if let Some(d) = dest.parent() {
        fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let dest_s = dest.display().to_string();
    let raw = wacli(
        &[
            "media",
            "download",
            "--id",
            id,
            "--chat",
            chat,
            "--output",
            dest_s.as_str(),
        ],
        false,
        Duration::from_secs(60),
    )?;
    if dest.is_file() {
        return Ok(());
    }
    // wacli may write into a directory
    if dest.parent().map(|p| p.is_dir()).unwrap_or(false) {
        if let Ok(rd) = fs::read_dir(dest.parent().unwrap()) {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_file() {
                    let _ = fs::rename(&p, dest);
                    if dest.is_file() {
                        return Ok(());
                    }
                }
            }
        }
    }
    let _ = raw;
    Err("media missing".into())
}

fn send_text(to: &str, text: &str, reply_to: Option<&str>) -> Result<(), String> {
    let mut extra = vec!["send", "text", "--to", to, "--message", text];
    if let Some(id) = reply_to {
        extra.extend(["--reply-to", id]);
    }
    wacli(&extra, false, Duration::from_secs(60))?;
    log_line("sent text");
    Ok(())
}

fn send_voice(to: &str, text: &str, reply_to: Option<&str>) -> Result<(), String> {
    let tongue = auto_tongue();
    let wav = cache_dir().join("out.wav");
    speak::synth_to(tongue, text, &wav).map_err(|e| e)?;
    if !wav.is_file() {
        return Err("synth produced no wav".into());
    }
    let note = wav_to_opus(&wav)?;
    let note_s = note.display().to_string();
    let mut extra = vec!["send", "voice", "--to", to, "--file", note_s.as_str()];
    if let Some(id) = reply_to {
        extra.extend(["--reply-to", id]);
    }
    wacli(&extra, false, Duration::from_secs(90))?;
    log_line("sent voice");
    Ok(())
}

fn auto_tongue() -> Tongue {
    let ui = prefs::load_ui().unwrap_or_default();
    let t = Tongue::from_str(&ui.speak_kind);
    if speak::ready_tongue(t) {
        t
    } else {
        Tongue::Design
    }
}

fn wav_to_opus(wav: &Path) -> Result<PathBuf, String> {
    let ext = wav
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if ext == "ogg" || ext == "opus" {
        return Ok(wav.to_path_buf());
    }
    let dest = wav.with_extension("ogg");
    let st = Command::new("ffmpeg")
        .args([
            "-y",
            "-i",
            &wav.display().to_string(),
            "-c:a",
            "libopus",
            "-b:a",
            "24k",
            "-ar",
            "16000",
            "-ac",
            "1",
            &dest.display().to_string(),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| format!("ffmpeg: {e}"))?;
    if !st.success() || !dest.is_file() {
        return Err("could not wrap the clip as opus".into());
    }
    Ok(dest)
}

fn list_new_messages(since: &str) -> Result<Vec<Value>, String> {
    let raw = wacli(
        &[
            "messages",
            "list",
            "--from-them",
            "--after",
            since,
            "--limit",
            "20",
        ],
        true,
        Duration::from_secs(20),
    )?;
    Ok(arr_at(&raw, "/data/messages"))
}

fn list_new_calls(since: &str) -> Result<Vec<Value>, String> {
    let raw = wacli(
        &["calls", "list", "--after", since, "--limit", "20"],
        true,
        Duration::from_secs(20),
    )?;
    Ok(arr_at(&raw, "/data/calls"))
}

fn arr_at(v: &Value, ptr: &str) -> Vec<Value> {
    v.pointer(ptr)
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default()
}

fn wacli(extra: &[&str], read_only: bool, timeout: Duration) -> Result<Value, String> {
    if extra.iter().any(|a| *a == "--follow") {
        return Err("follow is owned by the auto unit, not a one-shot".into());
    }
    let bin = wacli_bin();
    let mut cmd = Command::new(&bin);
    cmd.args(["--json", "--lock-wait", "30s"]);
    if read_only {
        cmd.arg("--read-only");
    }
    cmd.args(extra);
    cmd.stdin(Stdio::null());
    let out = run_timeout(cmd, timeout)?;
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let parsed: Value = serde_json::from_str(stdout.trim()).unwrap_or(json!({
        "stdout": stdout,
        "success": out.status.success(),
    }));
    if let Some(e) = parsed.get("error").and_then(|x| x.as_str()) {
        if !e.is_empty() && parsed.get("success").and_then(|x| x.as_bool()) == Some(false) {
            return Err(e.chars().take(180).collect());
        }
    }
    Ok(parsed)
}

fn run_timeout(mut cmd: Command, timeout: Duration) -> Result<std::process::Output, String> {
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                return child.wait_with_output().map_err(|e| e.to_string());
            }
            Ok(None) => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("wacli timed out".into());
                }
                std::thread::sleep(Duration::from_millis(40));
            }
            Err(e) => return Err(e.to_string()),
        }
    }
}

fn spawn_follow() -> Option<Child> {
    let log = match OpenOptions::new()
        .create(true)
        .append(true)
        .open(paths::cache_dir().join("wa-auto.log"))
    {
        Ok(f) => f,
        Err(_) => return None,
    };
    let err = match log.try_clone() {
        Ok(f) => f,
        Err(_) => return None,
    };
    Command::new(wacli_bin())
        .args(["sync", "--follow", "--presence-mode", "quiet"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(err))
        .spawn()
        .ok()
}

fn child_alive(child: &mut Option<Child>) -> bool {
    match child {
        None => false,
        Some(c) => match c.try_wait() {
            Ok(None) => true,
            Ok(Some(_)) => {
                *child = None;
                false
            }
            Err(_) => false,
        },
    }
}

fn kill_child(child: &mut Option<Child>) {
    if let Some(mut c) = child.take() {
        let _ = c.kill();
        let _ = c.wait();
    }
}

fn wacli_bin() -> PathBuf {
    let p = Path::new("/usr/bin/wacli");
    if p.is_file() {
        p.to_path_buf()
    } else {
        PathBuf::from("wacli")
    }
}

fn state_path() -> PathBuf {
    paths::cache_dir().join("wa-auto.json")
}

fn cache_dir() -> PathBuf {
    let p = paths::cache_dir().join("wa-auto");
    let _ = fs::create_dir_all(&p);
    p
}

fn load_state() -> State {
    let t = fs::read_to_string(state_path()).unwrap_or_default();
    let v: Value = serde_json::from_str(&t).unwrap_or(json!({}));
    let since = v
        .get("since")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let seen = v
        .get("seen")
        .and_then(|x| x.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    State { since, seen }
}

fn save_state(st: &State) {
    let _ = fs::create_dir_all(paths::cache_dir());
    let v = json!({
        "since": st.since,
        "seen": st.seen,
    });
    let _ = paths::write_atomic(&state_path(), v.to_string().as_bytes());
}

fn remember(st: &mut State, id: &str) {
    if st.seen.iter().any(|s| s == id) {
        return;
    }
    st.seen.push(id.to_string());
    if st.seen.len() > SEEN_CAP {
        let drop_n = st.seen.len() - SEEN_CAP;
        st.seen.drain(0..drop_n);
    }
}

fn rfc3339_now() -> String {
    Command::new("date")
        .args(["-u", "+%Y-%m-%dT%H:%M:%SZ"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "1970-01-01T00:00:00Z".into())
}

fn file_min(p: &Path, min: u64) -> bool {
    p.is_file() && fs::metadata(p).map(|m| m.len()).unwrap_or(0) >= min
}

fn safe_id(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c
            } else {
                '_'
            }
        })
        .take(40)
        .collect()
}

fn log_line(msg: &str) {
    let p = paths::cache_dir().join("wa-auto.log");
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(p) {
        let ts = mind::now_ts();
        let _ = writeln!(f, "{ts} {msg}");
    }
}

fn append_user(sid: &str, text: &str) {
    mind::append_session(
        sid,
        &SessionRec {
            ts: mind::now_ts(),
            role: "user".into(),
            rune: "auto".into(),
            text: text.to_string(),
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
}

const AUTO_PROMPT: &str = r#"You are Pixie covering WhatsApp. Gil cannot reach WhatsApp right now.
Competent first, then a little fairy. All-ages.
You have NO tools. Never emit JSON, tool names, bash, markdown fences, or <tool_call>.
Reply with the words to send, nothing else.
First ping in a chat: say you are Pixie, Gil is away from WhatsApp, offer help.
Later pings: help in words, still knowing Gil is away.
If they sent a file or picture: say you cannot view files yet, then continue.
Match their language if it is obvious, else English.
Do not invent machine work. Do not dump secrets, NIFs, client names, or mail.
The house sends your words as text or as a voice note. Keep voice replies to a short spoken paragraph."#;

fn auto_system_prompt() -> String {
    let host_card = host::probe();
    let mem = mind::load_memory_inject(None, mind::MEMORY_INJECT_CAP);
    format!("{AUTO_PROMPT}\n\nHost card:\n{host_card}\n\nHouse memory:\n{mem}")
}

fn auto_messages(chat: &[ChatEntry]) -> Vec<Value> {
    let mut msgs = vec![json!({"role": "system", "content": auto_system_prompt()})];
    let keep: Vec<&ChatEntry> = chat.iter().rev().take(24).collect();
    for e in keep.into_iter().rev() {
        match e {
            ChatEntry::User { text, .. } if !text.trim().is_empty() => {
                msgs.push(json!({"role": "user", "content": text}));
            }
            ChatEntry::Assistant { text, .. } if !text.trim().is_empty() => {
                msgs.push(json!({"role": "assistant", "content": text}));
            }
            _ => {}
        }
    }
    msgs
}

struct AutoOut {
    text: String,
    think: String,
    err: Option<String>,
}

fn run_auto_turn(chat: &[ChatEntry], abort: &Abort) -> AutoOut {
    if cfg!(test) {
        return AutoOut {
            text: FALLBACK.to_string(),
            think: String::new(),
            err: None,
        };
    }
    let msgs = auto_messages(chat);
    let out = llm::stream_chat(&msgs, None, abort, |_| {});
    if let Some(e) = out.err {
        return AutoOut {
            text: FALLBACK.to_string(),
            think: String::new(),
            err: Some(e),
        };
    }
    let split = think::split(&out.text);
    AutoOut {
        text: split.visible,
        think: split.think,
        err: None,
    }
}

fn append_assistant(sid: &str, text: &str, think: &str) {
    mind::append_session(
        sid,
        &SessionRec {
            ts: mind::now_ts(),
            role: "assistant".into(),
            rune: "auto".into(),
            text: text.to_string(),
            tool: String::new(),
            args: None,
            result: None,
            raw: String::new(),
            think: think.to_string(),
            cancelled: false,
            from_rune: String::new(),
            to_rune: String::new(),
            reason: String::new(),
        },
        None,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_text_voice_media_and_skips() {
        assert_eq!(
            classify_message(false, "1@s.whatsapp.net", "", "", "", false, false),
            Some(InboundKind::Text)
        );
        assert_eq!(
            classify_message(
                false,
                "1@s.whatsapp.net",
                "audio",
                "audio/ogg; codecs=opus",
                "",
                false,
                false
            ),
            Some(InboundKind::VoiceNote)
        );
        assert_eq!(
            classify_message(
                false,
                "1@s.whatsapp.net",
                "image",
                "image/jpeg",
                "",
                false,
                false
            ),
            Some(InboundKind::Media)
        );
        assert_eq!(
            classify_message(
                false,
                "1@s.whatsapp.net",
                "gif",
                "video/mp4",
                "",
                false,
                false
            ),
            Some(InboundKind::Media)
        );
        assert_eq!(
            classify_message(true, "1@s.whatsapp.net", "", "", "", false, false),
            None
        );
        assert_eq!(
            classify_message(false, "x@g.us", "", "", "", false, false),
            None
        );
        assert_eq!(
            classify_message(false, "1@s.whatsapp.net", "", "", "ABC", false, false),
            None
        );
        assert_eq!(
            classify_call("inbound", "missed"),
            Some(InboundKind::CallMissed)
        );
        assert_eq!(classify_call("inbound", "connected"), None);
        assert_eq!(classify_call("outbound", "missed"), None);
    }

    #[test]
    fn voice_in_burst_wins_reply_kind() {
        let text = Inbound {
            id: "a".into(),
            chat: "1@s.whatsapp.net".into(),
            kind: InboundKind::Text,
            text: "hi".into(),
            media_kind: String::new(),
            caption: String::new(),
            reply_to: Some("a".into()),
        };
        let voice = Inbound {
            id: "b".into(),
            chat: "1@s.whatsapp.net".into(),
            kind: InboundKind::VoiceNote,
            text: String::new(),
            media_kind: "audio".into(),
            caption: String::new(),
            reply_to: Some("b".into()),
        };
        let burst = merge_burst(
            Burst {
                chat: "1@s.whatsapp.net".into(),
                items: vec![text],
            },
            voice,
        );
        assert_eq!(reply_kind(&burst.items), ReplyKind::Voice);
        assert_eq!(burst.items.len(), 2);
    }

    #[test]
    fn spoken_reply_strips_tool_json() {
        let raw = r#"{"name":"execute_bash","arguments":{"command":"rm -rf /"}}"#;
        assert_eq!(spoken_reply(raw), FALLBACK);
        let mixed = "Hello there.\n{\"name\":\"check_whatsapp\",\"arguments\":{\"action\":\"send\"}}\n";
        let out = spoken_reply(mixed);
        assert!(out.contains("Hello there"), "{out}");
        assert!(!out.contains("check_whatsapp"), "{out}");
        assert!(!out.contains("execute_bash"), "{out}");
        let fenced = "```json\n{\"name\":\"look\",\"arguments\":{}}\n```";
        assert_eq!(spoken_reply(fenced), FALLBACK);
        assert_eq!(spoken_reply("  "), FALLBACK);
        assert_eq!(
            spoken_reply("Gil is away. I am Pixie — can I help?"),
            "Gil is away. I am Pixie — can I help?"
        );
    }

    #[test]
    fn follow_stays_out_of_oneshot_wacli() {
        let src = include_str!("wa_auto.rs");
        assert!(src.contains("sync\", \"--follow\""));
        assert!(src.contains("presence-mode"));
        assert!(src.contains("pixie-auto.service"));
        let err = wacli(&["sync", "--follow"], true, Duration::from_secs(1));
        assert!(err.is_err(), "{err:?}");
    }

    #[test]
    fn media_prompt_says_cannot_view() {
        let burst = Burst {
            chat: "1@s.whatsapp.net".into(),
            items: vec![Inbound {
                id: "m1".into(),
                chat: "1@s.whatsapp.net".into(),
                kind: InboundKind::Media,
                text: String::new(),
                media_kind: "image".into(),
                caption: "cat".into(),
                reply_to: Some("m1".into()),
            }],
        };
        let p = inbound_prompt(&burst, true);
        assert!(p.contains("cannot view files"), "{p}");
        assert!(p.contains("image"), "{p}");
        assert!(!p.contains("@s.whatsapp.net"), "{p}");
    }
}
