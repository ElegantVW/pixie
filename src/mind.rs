//! Spells, memory, sessions, house search, light web. No llama.

use crate::paths;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const MEMORY_INJECT_CAP: usize = 6000;
pub const MEMORY_FILE_CAP: usize = 24000;
pub const SESSION_TEXT_CAP: usize = 32000;
pub const SESSION_TOOL_CAP: usize = 1200;
pub const HOUSE_PACK_CAP: usize = 4000;

const SEED: &str = r#"# Pixie memory

- Host: office box. OS: Linux. Login: operator.
- Spells live in `~/bin`. Voice: cute after competence. Offline by default.
- Truth: docs in this repo + your shell config.
- Do not store client names, NIFs, keys, or passwords here.
"#;

const HOUSE_MD: &str = include_str!("../config/pixie/context/house.md");
const OS_MD: &str = include_str!("../config/pixie/context/os-commands.md");
const TOOLS_MD: &str = include_str!("../config/pixie/context/tools.md");
const VOICE_MD: &str = include_str!("../config/pixie/context/voice.md");

#[derive(Clone, Debug, Deserialize)]
pub struct Spell {
    pub bin: Option<String>,
    pub args: Option<Vec<String>>,
    pub confirm: Option<bool>,
    pub note: Option<String>,
}

#[derive(Deserialize)]
struct SpellsFile {
    spells: Option<HashMap<String, Spell>>,
}

pub fn load_spells() -> HashMap<String, Spell> {
    for p in spell_files() {
        if let Ok(text) = fs::read_to_string(&p) {
            if let Ok(file) = toml::from_str::<SpellsFile>(&text) {
                if let Some(s) = file.spells {
                    return s;
                }
            }
        }
    }
    HashMap::new()
}

fn spell_files() -> Vec<PathBuf> {
    let mut v = vec![
        paths::config_dir().join("spells.toml"),
        paths::home().join("faeOS/config/pixie/spells.toml"),
    ];
    if let Ok(manifest) = std::env::var("CARGO_MANIFEST_DIR") {
        v.push(PathBuf::from(manifest).join("config/pixie/spells.toml"));
    }
    v.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("config/pixie/spells.toml"));
    v
}

pub fn spell_names(spells: &HashMap<String, Spell>) -> Vec<String> {
    let mut n: Vec<_> = spells.keys().cloned().collect();
    n.sort();
    n
}

pub fn spells_line(spells: &HashMap<String, Spell>) -> String {
    let names = spell_names(spells);
    if names.is_empty() {
        "House spells: (none loaded).".into()
    } else {
        format!(
            "House spells (prefer the house tool): {}.",
            names.join(", ")
        )
    }
}

pub fn house_pack() -> String {
    let mut chunks = vec![
        HOUSE_MD.to_string(),
        OS_MD.to_string(),
        TOOLS_MD.to_string(),
        VOICE_MD.to_string(),
    ];
    let live = paths::config_dir().join("context");
    if live.is_dir() {
        let mut loaded = Vec::new();
        for name in ["house.md", "os-commands.md", "tools.md", "voice.md"] {
            let p = live.join(name);
            if let Ok(t) = fs::read_to_string(p) {
                loaded.push(t);
            }
        }
        if loaded.len() == 4 {
            chunks = loaded;
        }
    }
    let mut s = chunks.join("\n\n");
    if s.len() > HOUSE_PACK_CAP {
        s.truncate(HOUSE_PACK_CAP);
        s.push('…');
    }
    s
}

pub fn resolve_spell(
    name: &str,
    extra: &[String],
    spells: &HashMap<String, Spell>,
) -> Result<Resolved, String> {
    let spec = spells.get(name).ok_or_else(|| {
        let known = spell_names(spells).join(", ");
        format!("unknown spell '{name}'. Known: {known}")
    })?;
    let bin_name = spec.bin.clone().unwrap_or_else(|| name.to_string());
    let mut argv = Vec::new();
    argv.push(which(&bin_name).unwrap_or(bin_name));
    if let Some(prefix) = &spec.args {
        argv.extend(prefix.clone());
    }
    argv.extend(extra.iter().map(|a| expand_tilde(a)));
    Ok(Resolved {
        argv,
        confirm: spec.confirm.unwrap_or(false),
        note: spec.note.clone().unwrap_or_default(),
        spell: name.to_string(),
    })
}

pub struct Resolved {
    pub argv: Vec<String>,
    pub confirm: bool,
    #[allow(dead_code)]
    pub note: String,
    #[allow(dead_code)]
    pub spell: String,
}

pub fn needs_shell(command: &str) -> bool {
    let c = command.trim();
    c.contains("&&")
        || c.contains("||")
        || c.contains(';')
        || c.contains('|')
        || c.contains('$')
        || c.contains('`')
        || c.contains('>')
        || c.contains('<')
        || c.contains('\n')
        || c.contains("$( ")
        || (c.contains('*') && !c.starts_with("rg "))
}

pub fn rewrite_bash(command: &str, spells: &HashMap<String, Spell>) -> Result<Vec<String>, String> {
    let argv = split_argv(command)?;
    if argv.is_empty() {
        return Ok(argv);
    }
    let argv: Vec<String> = argv.into_iter().map(|a| expand_tilde(&a)).collect();
    if spells.contains_key(&argv[0]) {
        if let Ok(r) = resolve_spell(&argv[0], &argv[1..], spells) {
            return Ok(r.argv);
        }
    }
    Ok(argv)
}

pub fn run_argv(argv: &[String], timeout: Duration) -> Value {
    if argv.is_empty() {
        return json!({"error": "empty command"});
    }
    let mut cmd = Command::new(&argv[0]);
    if argv.len() > 1 {
        cmd.args(&argv[1..]);
    }
    cmd.env("PATH", paths::house_path());
    apply_sudo_env(&mut cmd);
    cmd.stdin(Stdio::null());
    match spawn_timeout(cmd, timeout, None) {
        SpawnOut::Err(e) => json!({"error": e}),
        SpawnOut::Ok(o) => json!({
            "stdout": String::from_utf8_lossy(&o.stdout).trim().to_string(),
            "stderr": String::from_utf8_lossy(&o.stderr).trim().to_string(),
            "exit_code": o.status.code().unwrap_or(-1),
            "argv": argv,
        }),
    }
}

pub fn run_argv_cancel(
    argv: &[String],
    timeout: Duration,
    cancel: &std::sync::atomic::AtomicBool,
) -> Value {
    if argv.is_empty() {
        return json!({"error": "empty command"});
    }
    let mut cmd = Command::new(&argv[0]);
    if argv.len() > 1 {
        cmd.args(&argv[1..]);
    }
    cmd.env("PATH", paths::house_path());
    apply_sudo_env(&mut cmd);
    cmd.stdin(Stdio::null());
    match spawn_timeout(cmd, timeout, Some(cancel)) {
        SpawnOut::Err(e) => json!({"error": e, "argv": argv}),
        SpawnOut::Ok(o) => json!({
            "stdout": String::from_utf8_lossy(&o.stdout).trim().to_string(),
            "stderr": String::from_utf8_lossy(&o.stderr).trim().to_string(),
            "exit_code": o.status.code().unwrap_or(-1),
            "argv": argv,
        }),
    }
}

fn apply_sudo_env(cmd: &mut Command) {
    if let Some((k, v)) = crate::privilege::child_env() {
        cmd.env(k, v);
    } else {
        cmd.env("SUDO_ASKPASS", "/bin/false");
    }
}

enum SpawnOut {
    Ok(std::process::Output),
    Err(String),
}

#[cfg(unix)]
extern "C" {
    fn setsid() -> i32;
    fn kill(pid: i32, sig: i32) -> i32;
}

fn stop_child_group(child: &mut std::process::Child) {
    #[cfg(unix)]
    unsafe {
        let _ = kill(-(child.id() as i32), 9);
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn spawn_timeout(
    mut cmd: Command,
    timeout: Duration,
    cancel: Option<&std::sync::atomic::AtomicBool>,
) -> SpawnOut {
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            cmd.pre_exec(|| {
                if setsid() < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    let mut child = match cmd.spawn() {
        Err(e) => {
            return SpawnOut::Err(if e.kind() == std::io::ErrorKind::NotFound {
                format!("command not found: {}", cmd.get_program().to_string_lossy())
            } else {
                e.to_string()
            });
        }
        Ok(c) => c,
    };
    let start = std::time::Instant::now();
    let status = loop {
        if cancel
            .map(|c| c.load(std::sync::atomic::Ordering::SeqCst))
            .unwrap_or(false)
        {
            stop_child_group(&mut child);
            return SpawnOut::Err("cancelled".into());
        }
        match child.try_wait() {
            Ok(Some(st)) => {
                #[cfg(unix)]
                unsafe {
                    let _ = kill(-(child.id() as i32), 9);
                }
                break st;
            }
            Ok(None) => {
                if start.elapsed() > timeout {
                    stop_child_group(&mut child);
                    return SpawnOut::Err(format!("timed out after {}s", timeout.as_secs()));
                }
                std::thread::sleep(Duration::from_millis(40));
            }
            Err(e) => {
                stop_child_group(&mut child);
                return SpawnOut::Err(e.to_string());
            }
        }
    };
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    if let Some(mut s) = child.stdout.take() {
        let _ = std::io::Read::read_to_end(&mut s, &mut stdout);
    }
    if let Some(mut s) = child.stderr.take() {
        let _ = std::io::Read::read_to_end(&mut s, &mut stderr);
    }
    SpawnOut::Ok(std::process::Output {
        status,
        stdout,
        stderr,
    })
}

pub fn seed_memory(path: Option<&Path>) -> anyhow::Result<PathBuf> {
    let p = path
        .map(|p| p.to_path_buf())
        .unwrap_or_else(paths::memory_path);
    if let Some(d) = p.parent() {
        fs::create_dir_all(d)
            .map_err(|e| anyhow::anyhow!("creating memory directory {}: {e}", d.display()))?;
    }
    match fs::read_to_string(&p) {
        Ok(contents) if contents.trim().is_empty() => {
            paths::write_atomic(&p, SEED.as_bytes())
                .map_err(|e| anyhow::anyhow!("seeding memory {}: {e}", p.display()))?;
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            paths::write_atomic(&p, SEED.as_bytes())
                .map_err(|e| anyhow::anyhow!("seeding memory {}: {e}", p.display()))?;
        }
        Err(error) => return Err(anyhow::anyhow!("reading memory {}: {error}", p.display())),
    }
    Ok(p)
}

pub fn load_memory_inject(path: Option<&Path>, cap: usize) -> String {
    let p = match seed_memory(path) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("pixie: could not load house memory: {error}");
            return String::new();
        }
    };
    let mut text = match fs::read_to_string(&p) {
        Ok(text) => text,
        Err(error) => {
            eprintln!(
                "pixie: could not read house memory {}: {error}",
                p.display()
            );
            return String::new();
        }
    };
    if text.len() > cap {
        text.truncate(cap);
        text.push('…');
    }
    text
}

pub fn remember(fact: &str, path: Option<&Path>) -> Value {
    let fact: String = fact.split_whitespace().collect::<Vec<_>>().join(" ");
    if fact.is_empty() {
        return json!({"error": "empty fact"});
    }
    let p = match seed_memory(path) {
        Ok(path) => path,
        Err(error) => return json!({"error": format!("could not load memory: {error}")}),
    };
    let mut body = match fs::read_to_string(&p) {
        Ok(body) => body,
        Err(error) => return json!({"error": format!("could not read {}: {error}", p.display())}),
    };
    if body.to_lowercase().contains(&fact.to_lowercase()) {
        return json!({"path": p.display().to_string(), "wrote": false, "note": "already known"});
    }
    let today = chrono_today();
    body = body.trim_end().to_string();
    body.push('\n');
    body.push_str(&format!("- {today}: {fact}\n"));
    if body.len() > MEMORY_FILE_CAP {
        let lines: Vec<&str> = body.split_inclusive('\n').collect();
        let mut head = Vec::new();
        let mut rest = Vec::new();
        for ln in lines {
            if ln.starts_with("- ") && !rest.is_empty() {
                rest.push(ln);
            } else if !rest.is_empty() || ln.starts_with("- ") {
                rest.push(ln);
            } else {
                head.push(ln);
            }
        }
        while !rest.is_empty() {
            let n: usize = head.iter().map(|s| s.len()).sum::<usize>()
                + rest.iter().map(|s| s.len()).sum::<usize>();
            if n <= MEMORY_FILE_CAP {
                break;
            }
            rest.remove(0);
        }
        body = head.concat() + &rest.concat();
    }
    if let Err(error) = paths::write_atomic(&p, body.as_bytes()) {
        return json!({"error": format!("could not save {}: {error}", p.display())});
    }
    json!({"path": p.display().to_string(), "wrote": true})
}

fn chrono_today() -> String {
    // local YYYY-MM-DD without extra crates
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // crude UTC date; good enough for memory stamps
    let days = secs / 86400;
    let mut y = 1970u32;
    let mut rem = days;
    loop {
        let ly = y.is_multiple_of(4) && (!y.is_multiple_of(100) || y.is_multiple_of(400));
        let yl = if ly { 366 } else { 365 };
        if rem >= yl {
            rem -= yl;
            y += 1;
        } else {
            break;
        }
    }
    let ly = y.is_multiple_of(4) && (!y.is_multiple_of(100) || y.is_multiple_of(400));
    let md = [
        31,
        if ly { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut m = 1u32;
    for d in md {
        if rem >= d {
            rem -= d;
            m += 1;
        } else {
            break;
        }
    }
    format!("{y:04}-{m:02}-{:02}", rem + 1)
}

fn cap_value(v: &Value, cap: usize) -> Value {
    let s = v.to_string();
    if s.len() <= cap {
        v.clone()
    } else {
        json!({"_truncated": true, "preview": format!("{}…", &s[..cap])})
    }
}

fn cap_str(s: &str, cap: usize) -> String {
    if s.len() <= cap {
        s.to_string()
    } else {
        format!("{}…", &s[..cap])
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SessionRec {
    pub ts: String,
    pub role: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub rune: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tool: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub raw: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub think: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cancelled: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub from_rune: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub to_rune: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reason: String,
}

impl Default for SessionRec {
    fn default() -> Self {
        Self {
            ts: String::new(),
            role: String::new(),
            rune: String::new(),
            text: String::new(),
            tool: String::new(),
            args: None,
            result: None,
            raw: String::new(),
            think: String::new(),
            cancelled: false,
            from_rune: String::new(),
            to_rune: String::new(),
            reason: String::new(),
        }
    }
}

pub fn session_path(id: &str, dir: Option<&Path>) -> PathBuf {
    dir.map(|d| d.to_path_buf())
        .unwrap_or_else(paths::sessions_dir)
        .join(format!("{id}.jsonl"))
}

pub fn append_transition(
    id: &str,
    from_rune: &str,
    to_rune: &str,
    reason: &str,
    text: &str,
    dir: Option<&std::path::Path>,
) {
    append_session(
        id,
        &SessionRec {
            ts: now_ts(),
            role: "transition".into(),
            rune: to_rune.into(),
            text: text.into(),
            tool: String::new(),
            args: None,
            result: None,
            raw: String::new(),
            think: String::new(),
            cancelled: false,
            from_rune: from_rune.into(),
            to_rune: to_rune.into(),
            reason: reason.into(),
        },
        dir,
    );
}

pub fn last_transition(id: &str, dir: Option<&Path>) -> Option<(String, String, String, String)> {
    let recs = read_session(id, dir);
    for rec in recs.into_iter().rev() {
        if rec.get("role").and_then(|x| x.as_str()) == Some("transition") {
            let from = rec
                .get("from_rune")
                .and_then(|x| x.as_str())
                .or_else(|| rec.get("from").and_then(|x| x.as_str()))
                .unwrap_or("")
                .to_string();
            let to = rec
                .get("to_rune")
                .and_then(|x| x.as_str())
                .or_else(|| rec.get("to").and_then(|x| x.as_str()))
                .unwrap_or("")
                .to_string();
            let reason = rec
                .get("reason")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            let text = rec
                .get("text")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            if !from.is_empty() || !to.is_empty() {
                return Some((from, to, reason, text));
            }
        }
    }
    None
}

pub fn append_session(id: &str, rec: &SessionRec, dir: Option<&Path>) {
    let result = (|| -> anyhow::Result<()> {
        let dir = dir
            .map(|d| d.to_path_buf())
            .unwrap_or_else(paths::sessions_dir);
        fs::create_dir_all(&dir).map_err(|e| anyhow::anyhow!("creating {}: {e}", dir.display()))?;
        let p = dir.join(format!("{id}.jsonl"));
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&p)
            .map_err(|e| anyhow::anyhow!("opening {}: {e}", p.display()))?;
        let mut rec = rec.clone();
        rec.text = cap_str(&rec.text, SESSION_TEXT_CAP);
        rec.raw = cap_str(&rec.raw, SESSION_TOOL_CAP);
        if let Some(a) = rec.args.take() {
            rec.args = Some(cap_value(&a, SESSION_TOOL_CAP));
        }
        if let Some(r) = rec.result.take() {
            rec.result = Some(cap_value(&r, SESSION_TOOL_CAP));
        }
        let line = serde_json::to_string(&rec)
            .map_err(|e| anyhow::anyhow!("encoding session record: {e}"))?;
        writeln!(f, "{line}").map_err(|e| anyhow::anyhow!("writing {}: {e}", p.display()))?;
        Ok(())
    })();
    if let Err(error) = result {
        eprintln!("pixie: could not save session record for {id}: {error}");
    }
}

pub fn now_ts() -> String {
    // ISO-ish local via `date`
    Command::new("date")
        .args(["-Iseconds"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "now".into())
}

pub fn read_session(id: &str, dir: Option<&Path>) -> Vec<Value> {
    let p = session_path(id, dir);
    let Ok(text) = fs::read_to_string(p) else {
        return Vec::new();
    };
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

pub fn list_sessions(dir: Option<&Path>) -> Vec<Value> {
    let d = dir
        .map(|d| d.to_path_buf())
        .unwrap_or_else(paths::sessions_dir);
    let Ok(rd) = fs::read_dir(&d) else {
        return Vec::new();
    };
    let mut rows: Vec<(u64, Value)> = Vec::new();
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) != Some("jsonl") {
            continue;
        }
        let id = p
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        let mtime = e
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let recs = read_session(&id, Some(&d));
        let mut user = 0;
        let mut assistant = 0;
        let mut tool = 0;
        let mut first_user = String::new();
        for r in &recs {
            match r.get("role").and_then(|x| x.as_str()).unwrap_or("") {
                "user" => {
                    user += 1;
                    if first_user.is_empty() {
                        first_user = r
                            .get("text")
                            .and_then(|x| x.as_str())
                            .unwrap_or("")
                            .replace('\n', " ")
                            .chars()
                            .take(80)
                            .collect();
                    }
                }
                "assistant" => assistant += 1,
                "tool" => tool += 1,
                _ => {}
            }
        }
        rows.push((
            mtime,
            json!({
                "id": id,
                "path": p.display().to_string(),
                "mtime": mtime,
                "first_user": first_user,
                "user": user,
                "assistant": assistant,
                "tool": tool,
            }),
        ));
    }
    rows.sort_by(|a, b| b.0.cmp(&a.0));
    rows.into_iter().map(|(_, v)| v).collect()
}

pub fn latest_session_id(dir: Option<&Path>) -> Option<String> {
    list_sessions(dir)
        .first()
        .and_then(|r| r.get("id").and_then(|x| x.as_str()).map(|s| s.to_string()))
}

#[derive(Clone, Debug)]
pub struct SessionRow {
    pub id: String,
    pub name: String,
    pub summary: String,
    pub users: u64,
    pub assistants: u64,
    pub mtime: u64,
}

/// Newest first. Name + blurb for the dashboard.
pub fn session_previews(dir: Option<&Path>) -> Vec<SessionRow> {
    list_sessions(dir)
        .iter()
        .filter_map(|r| {
            let id = r
                .get("id")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            if id.is_empty() {
                return None;
            }
            let meta = load_meta(&id, dir);
            let name = if !meta.name.is_empty() {
                meta.name.clone()
            } else {
                id.clone()
            };
            let first = preview_safe(r.get("first_user").and_then(|x| x.as_str()).unwrap_or(""));
            let summary = if !meta.summary.is_empty() {
                preview_safe(&meta.summary)
            } else if let Some(c) = last_compact_text(&id, dir) {
                preview_safe(&c)
            } else {
                first
            };
            Some(SessionRow {
                id,
                name,
                summary,
                users: r.get("user").and_then(|x| x.as_u64()).unwrap_or(0),
                assistants: r.get("assistant").and_then(|x| x.as_u64()).unwrap_or(0),
                mtime: r.get("mtime").and_then(|x| x.as_u64()).unwrap_or(0),
            })
        })
        .collect()
}

pub fn age_label(mtime: u64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let d = now.saturating_sub(mtime);
    if d < 120 {
        "now".into()
    } else if d < 3600 {
        format!("{}m", d / 60)
    } else if d < 86400 {
        format!("{}h", d / 3600)
    } else if d < 86400 * 7 {
        format!("{}d", d / 86400)
    } else {
        "older".into()
    }
}

pub fn last_compact_text(id: &str, dir: Option<&Path>) -> Option<String> {
    read_session(id, dir)
        .into_iter()
        .rev()
        .find(|r| r.get("role").and_then(|x| x.as_str()) == Some("compact"))
        .and_then(|r| {
            r.get("text")
                .and_then(|x| x.as_str())
                .map(|s| s.to_string())
        })
        .filter(|s| !s.trim().is_empty())
}

pub fn needs_summary(id: &str, dir: Option<&Path>) -> bool {
    load_meta(id, dir).summary.trim().is_empty() && last_compact_text(id, dir).is_none()
}

pub fn set_summary(id: &str, text: &str, dir: Option<&Path>) {
    let mut m = load_meta(id, dir);
    let text = preview_safe(text);
    if text.is_empty() {
        return;
    }
    m.summary = text.chars().take(80).collect();
    save_meta(id, &m, dir);
}

pub fn hydrate_compact_blurbs(dir: Option<&Path>) {
    for row in session_previews(dir) {
        if !load_meta(&row.id, dir).summary.trim().is_empty() {
            continue;
        }
        if let Some(c) = last_compact_text(&row.id, dir) {
            set_summary(&row.id, &c, dir);
        }
    }
}

pub fn delete_session(id: &str, dir: Option<&Path>) -> bool {
    if id.trim().is_empty() || id.contains('/') || id.contains("..") {
        return false;
    }
    let jsonl = session_path(id, dir);
    let meta = meta_path(id, dir);
    let mut ok = false;
    if jsonl.is_file() {
        ok = fs::remove_file(&jsonl).is_ok();
    }
    if meta.is_file() {
        let _ = fs::remove_file(&meta);
    }
    ok
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionMeta {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub renamed: bool,
    #[serde(default)]
    pub named_from: usize,
    #[serde(default)]
    pub summary: String,
}

impl SessionMeta {
    fn for_id(id: &str) -> Self {
        Self {
            name: id.to_string(),
            renamed: false,
            named_from: 0,
            summary: String::new(),
        }
    }
}

pub fn meta_path(id: &str, dir: Option<&Path>) -> PathBuf {
    session_path(id, dir).with_extension("meta.json")
}

pub fn load_meta(id: &str, dir: Option<&Path>) -> SessionMeta {
    let p = meta_path(id, dir);
    fs::read_to_string(&p)
        .ok()
        .and_then(|t| serde_json::from_str::<SessionMeta>(&t).ok())
        .map(|mut m| {
            if m.name.trim().is_empty() {
                m.name = id.to_string();
            }
            m
        })
        .unwrap_or_else(|| SessionMeta::for_id(id))
}

pub fn save_meta(id: &str, meta: &SessionMeta, dir: Option<&Path>) {
    let p = meta_path(id, dir);
    if let Some(d) = p.parent() {
        if let Err(error) = fs::create_dir_all(d) {
            eprintln!(
                "pixie: could not create session metadata directory {}: {error}",
                d.display()
            );
            return;
        }
    }
    match serde_json::to_string_pretty(meta) {
        Ok(contents) => {
            if let Err(error) = paths::write_atomic(&p, contents.as_bytes()) {
                eprintln!(
                    "pixie: could not save session metadata {}: {error}",
                    p.display()
                );
            }
        }
        Err(error) => eprintln!("pixie: could not encode session metadata: {error}"),
    }
}

pub fn rename_session(id: &str, name: &str, dir: Option<&Path>) {
    let mut m = load_meta(id, dir);
    let name = name.trim();
    if name.is_empty() {
        return;
    }
    m.name = name.chars().take(40).collect();
    m.renamed = true;
    save_meta(id, &m, dir);
}

/// True if kur should retitle (under 10 user lines, not user-renamed).
pub fn should_retitle(id: &str, dir: Option<&Path>) -> bool {
    let m = load_meta(id, dir);
    !m.renamed && m.named_from < 10
}

pub fn bump_named_from(id: &str, dir: Option<&Path>) -> usize {
    let mut m = load_meta(id, dir);
    if m.renamed {
        return m.named_from;
    }
    m.named_from = m.named_from.saturating_add(1).min(10);
    save_meta(id, &m, dir);
    m.named_from
}

pub fn set_session_name(id: &str, name: &str, dir: Option<&Path>) {
    let mut m = load_meta(id, dir);
    if m.renamed {
        return;
    }
    let name = name.trim();
    if name.is_empty() || is_red_fact(name) {
        return;
    }
    m.name = name.chars().take(40).collect();
    save_meta(id, &m, dir);
}

pub fn user_texts(id: &str, dir: Option<&Path>, limit: usize) -> Vec<String> {
    read_session(id, dir)
        .into_iter()
        .filter(|r| r.get("role").and_then(|x| x.as_str()) == Some("user"))
        .filter_map(|r| {
            r.get("text")
                .and_then(|x| x.as_str())
                .map(|s| s.to_string())
        })
        .filter(|s| !s.trim().is_empty())
        .take(limit)
        .collect()
}

/// Bare `pixie` opens the door. `--new` / `--resume` skip it.
pub fn wants_door(new: bool, resume: bool) -> bool {
    !new && !resume
}

/// Last few you/Pixie lines, clipped so kur's tiny ctx can recap them.
pub fn compact_seed(lines: &[(String, String)]) -> String {
    let start = lines.len().saturating_sub(4);
    let mut out = String::new();
    for (role, text) in &lines[start..] {
        if text.trim().is_empty() {
            continue;
        }
        let clip: String = text.chars().take(220).collect();
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(role);
        out.push_str(": ");
        out.push_str(&clip);
    }
    out
}

pub(crate) fn preview_safe(s: &str) -> String {
    let t = s.trim();
    if t.is_empty() {
        return String::new();
    }
    if is_red_fact(t) {
        return "(house note)".into();
    }
    t.chars().take(80).collect()
}

pub(crate) fn is_red_fact(s: &str) -> bool {
    let l = s.to_lowercase();
    [
        "wifi",
        "wi-fi",
        "essid",
        "bssid",
        "aircrack",
        "airodump",
        "aireplay",
        "handshake",
        "password",
        "passwd",
        "nif",
        ".pem",
        "wpa-",
        "wep",
    ]
    .iter()
    .any(|k| l.contains(k))
}

/// Mid-thread compact blob: keep recap lines, drop secrets and radio.
pub fn scrub_compact(raw: &str) -> String {
    let mut out = String::new();
    for l in raw.lines() {
        let l = l.trim();
        if l.is_empty() || l.eq_ignore_ascii_case("none") || is_red_fact(l) || l.starts_with('{') {
            continue;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(l);
        if out.lines().count() >= 40 || out.len() >= 4000 {
            break;
        }
    }
    out
}

/// Parse a kur recap into durable house bullets. Drops secrets and radio.
pub fn durable_facts(raw: &str) -> Vec<String> {
    let t = raw.trim();
    if t.is_empty() || t.eq_ignore_ascii_case("none") {
        return Vec::new();
    }
    t.lines()
        .map(|l| {
            l.trim()
                .trim_start_matches(['-', '*', '•'])
                .trim()
                .trim_matches('"')
                .to_string()
        })
        .filter(|l| !l.is_empty())
        .filter(|l| !l.eq_ignore_ascii_case("none"))
        .filter(|l| !is_red_fact(l))
        .filter(|l| !l.starts_with('{'))
        .take(8)
        .collect()
}

pub fn resolve_session_id(sid: Option<&str>, dir: Option<&Path>) -> Option<String> {
    if let Some(s) = sid {
        let name = Path::new(s)
            .file_stem()
            .and_then(|x| x.to_str())
            .unwrap_or(s);
        let p = session_path(name, dir);
        if p.is_file() {
            return Some(name.to_string());
        }
        return None;
    }
    latest_session_id(dir)
}

pub fn format_chat_list(dir: Option<&Path>) -> String {
    let rows = list_sessions(dir);
    if rows.is_empty() {
        return "pixie: no chats stored yet".into();
    }
    rows.iter()
        .map(|r| {
            let id = r.get("id").and_then(|x| x.as_str()).unwrap_or("?");
            let u = r.get("user").and_then(|x| x.as_u64()).unwrap_or(0);
            let a = r.get("assistant").and_then(|x| x.as_u64()).unwrap_or(0);
            let t = r.get("tool").and_then(|x| x.as_u64()).unwrap_or(0);
            let preview = preview_safe(r.get("first_user").and_then(|x| x.as_str()).unwrap_or(""));
            let preview = if preview.is_empty() {
                "(empty)".into()
            } else {
                preview
            };
            let meta = load_meta(id, dir);
            let name = if !meta.name.is_empty() && meta.name != id {
                format!("{}  ", meta.name)
            } else {
                String::new()
            };
            format!("{name}{id}  u:{u} a:{a} t:{t}  {preview}")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn format_chat(id: &str, dir: Option<&Path>) -> String {
    let recs = read_session(id, dir);
    if recs.is_empty() {
        return format!("pixie: no chat named {id}");
    }
    let mut blocks = Vec::new();
    for rec in recs {
        let ts = rec.get("ts").and_then(|x| x.as_str()).unwrap_or("");
        let role = rec.get("role").and_then(|x| x.as_str()).unwrap_or("");
        let rune = rec.get("rune").and_then(|x| x.as_str()).unwrap_or("");
        let mut head = if ts.is_empty() {
            "[]".into()
        } else {
            format!("[{ts}]")
        };
        if !rune.is_empty() {
            head.push(' ');
            head.push_str(rune);
        }
        match role {
            "user" => blocks.push(format!(
                "{head} you: {}",
                rec.get("text").and_then(|x| x.as_str()).unwrap_or("")
            )),
            "assistant" => {
                let mut line = format!(
                    "{head} pixie: {}",
                    rec.get("text").and_then(|x| x.as_str()).unwrap_or("")
                );
                if rec
                    .get("cancelled")
                    .and_then(|x| x.as_bool())
                    .unwrap_or(false)
                {
                    line.push_str("  (cancelled)");
                }
                if let Some(th) = rec.get("think").and_then(|x| x.as_str()) {
                    if !th.is_empty() {
                        line.push_str("\n  think: ");
                        line.push_str(th);
                    }
                }
                blocks.push(line);
            }
            "compact" => {
                let text = rec.get("text").and_then(|x| x.as_str()).unwrap_or("");
                blocks.push(format!("{head} compact: {text}"));
            }
            "tool" => {
                let name = rec.get("tool").and_then(|x| x.as_str()).unwrap_or("tool");
                let mut line = format!("{head} tool {name}");
                if let Some(args) = rec.get("args") {
                    line.push(' ');
                    line.push_str(&args.to_string());
                }
                if let Some(raw) = rec.get("raw").and_then(|x| x.as_str()) {
                    if !raw.is_empty() {
                        line.push_str("\n  raw: ");
                        line.push_str(raw);
                    }
                }
                if let Some(result) = rec.get("result") {
                    line.push_str("\n  -> ");
                    line.push_str(&result.to_string());
                }
                blocks.push(line);
            }
            _ => blocks.push(format!(
                "{head} {role}: {}",
                rec.get("text").and_then(|x| x.as_str()).unwrap_or("")
            )),
        }
    }
    blocks.join("\n\n")
}

pub fn search_house(query: &str, max_hits: usize) -> Value {
    let query = query.trim();
    if query.is_empty() {
        return json!({"error": "empty query"});
    }
    let home = paths::home();
    let roots = [
        home.join("faeOS/docs"),
        home.join("faeOS/faeOSplan.md"),
        home.join("pixie/docs"),
        home.join("pixie/src"),
        home.join("pixie/config"),
        home.join("menagerie/src"),
        home.join("menagerie/README.md"),
        home.join("goblin/src"),
        home.join("imp/src"),
        paths::memory_path(),
    ];
    let mut hits = Vec::new();
    if let Some(rg) = which("rg") {
        for root in &roots {
            if !root.exists() {
                continue;
            }
            let out = Command::new(&rg)
                .args([
                    "-n",
                    "-i",
                    "--max-count",
                    "3",
                    "--no-heading",
                    "-g",
                    "!*.gguf",
                    query,
                    &root.display().to_string(),
                ])
                .output();
            if let Ok(o) = out {
                for line in String::from_utf8_lossy(&o.stdout).lines() {
                    if let Some((path, rest)) = line.split_once(':') {
                        hits.push(json!({"path": path, "line": rest.chars().take(200).collect::<String>()}));
                        if hits.len() >= max_hits {
                            return json!({"query": query, "hits": hits, "count": hits.len()});
                        }
                    }
                }
            }
        }
    } else {
        let needle = query.to_lowercase();
        let mut files = Vec::new();
        for root in &roots {
            if root.is_file() {
                files.push(root.clone());
            } else if root.is_dir() {
                walk_md(root, &mut files);
            }
        }
        'outer: for p in files {
            let Ok(text) = fs::read_to_string(&p) else {
                continue;
            };
            for (i, ln) in text.lines().enumerate() {
                if ln.to_lowercase().contains(&needle) {
                    hits.push(json!({
                        "path": p.display().to_string(),
                        "line": format!("{}:{}", i + 1, ln.trim().chars().take(180).collect::<String>())
                    }));
                    if hits.len() >= max_hits {
                        break 'outer;
                    }
                }
            }
        }
    }
    json!({"query": query, "hits": hits, "count": hits.len()})
}

fn walk_md(root: &Path, files: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(root) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk_md(&p, files);
        } else if p.extension().and_then(|x| x.to_str()) == Some("md")
            || p.extension().and_then(|x| x.to_str()) == Some("rs")
            || p.extension().and_then(|x| x.to_str()) == Some("toml")
        {
            files.push(p);
        }
    }
}

pub fn wiki(query: &str, max_results: i64) -> Value {
    let query = query.trim();
    if query.is_empty() {
        return json!({"error": "empty query"});
    }
    let url = format!(
        "https://en.wikipedia.org/w/api.php?action=query&list=search&srsearch={}&srlimit={}&format=json",
        urlencode(query),
        max_results.max(1).min(8)
    );
    match curl_get(&url, 20) {
        Err(e) => {
            json!({"query": query, "engine": "wikipedia", "results": [], "count": 0, "error": e})
        }
        Ok(body) => {
            let v: Value = serde_json::from_str(&body).unwrap_or(json!({}));
            let mut items = Vec::new();
            if let Some(arr) = v.pointer("/query/search").and_then(|x| x.as_array()) {
                for r in arr {
                    let title = r.get("title").and_then(|x| x.as_str()).unwrap_or("");
                    let snippet =
                        strip_tags(r.get("snippet").and_then(|x| x.as_str()).unwrap_or(""));
                    items.push(json!({
                        "title": title,
                        "url": format!("https://en.wikipedia.org/wiki/{}", title.replace(' ', "_")),
                        "snippet": snippet,
                    }));
                }
            }
            json!({"query": query, "engine": "wikipedia", "results": items, "count": items.len()})
        }
    }
}

pub fn search_web(query: &str, max_results: i64) -> Value {
    let query = query.trim();
    if query.is_empty() {
        return json!({"error": "empty query"});
    }
    let url = format!("https://html.duckduckgo.com/html/?q={}", urlencode(query));
    match curl_get(&url, 25) {
        Err(e) => json!({
            "query": query, "results": [], "count": 0,
            "error": e, "note": "search engine unavailable (offline?)"
        }),
        Ok(html) => {
            let items = parse_ddg(&html, max_results.max(1).min(8) as usize);
            if items.is_empty() {
                json!({
                    "query": query, "results": [], "count": 0,
                    "error": "no results", "note": "offline or engines blocked"
                })
            } else {
                json!({"query": query, "engine": "ddg-html", "results": items, "count": items.len()})
            }
        }
    }
}

fn parse_ddg(html: &str, max: usize) -> Vec<Value> {
    let mut items = Vec::new();
    let mut rest = html;
    while items.len() < max {
        let Some(i) = rest.find("uddg=") else { break };
        let after = &rest[i + 5..];
        let end = after.find('"').unwrap_or(after.len().min(400));
        let raw = &after[..end];
        let url = decode_url(raw);
        let rest2 = &after[end..];
        let title = {
            let t0 = rest2.find('>').map(|x| x + 1).unwrap_or(0);
            let t1 = rest2[t0..].find('<').unwrap_or(80);
            strip_tags(&rest2[t0..t0 + t1]).trim().to_string()
        };
        if url.starts_with("http") {
            items.push(json!({
                "title": if title.is_empty() { url.clone() } else { title },
                "url": url,
                "snippet": "",
            }));
        }
        rest = rest2;
    }
    items
}

pub fn read_page(url: &str, max_chars: usize) -> Value {
    let url = url.trim();
    if url.is_empty() {
        return json!({"error": "empty url"});
    }
    let magpie = which("magpie").unwrap_or_else(|| "magpie".into());
    let out = Command::new(&magpie)
        .args(["browse", "--dump", url])
        .env("PATH", paths::house_path())
        .output();
    match out {
        Err(e) => json!({"error": format!("magpie: {e}"), "url": url}),
        Ok(o) => {
            let mut text = String::from_utf8_lossy(&o.stdout).to_string();
            if text.trim().is_empty() {
                text = String::from_utf8_lossy(&o.stderr).to_string();
            }
            if text.len() > max_chars {
                text.truncate(max_chars);
                text.push('…');
            }
            json!({"url": url, "title": "", "text": text})
        }
    }
}

pub fn note_source(session_id: &str, url: &str, title: &str) {
    if session_id.is_empty() || url.is_empty() {
        return;
    }
    let dir = paths::research_dir();
    let _ = fs::create_dir_all(&dir);
    let p = dir.join(format!("{session_id}.jsonl"));
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(p) {
        let _ = writeln!(f, "{}", json!({"url": url, "title": title}));
    }
}

pub fn listed_sources(session_id: &str) -> Vec<Value> {
    let p = paths::research_dir().join(format!("{session_id}.jsonl"));
    let Ok(text) = fs::read_to_string(p) else {
        return Vec::new();
    };
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for line in text.lines() {
        if let Ok(v) = serde_json::from_str::<Value>(line) {
            if let Some(url) = v.get("url").and_then(|x| x.as_str()) {
                if seen.insert(url.to_string()) {
                    out.push(v);
                }
            }
        }
    }
    out
}

pub fn write_plan(
    goal: &str,
    steps: &Value,
    files: &Value,
    risks: &Value,
    verify: &str,
    session_id: &str,
) -> Value {
    let goal: String = goal.split_whitespace().collect::<Vec<_>>().join(" ");
    if goal.is_empty() {
        return json!({"error": "empty goal"});
    }
    let sid = if session_id.is_empty() {
        now_ts().replace([':', '+'], "-")
    } else {
        session_id.to_string()
    };
    write_plan_in(
        &paths::plans_dir(),
        &sid,
        &goal,
        steps,
        files,
        risks,
        verify,
    )
}

fn write_plan_in(
    dir: &Path,
    sid: &str,
    goal: &str,
    steps: &Value,
    files: &Value,
    risks: &Value,
    verify: &str,
) -> Value {
    if let Err(e) = fs::create_dir_all(dir) {
        return json!({
            "error": format!("could not create plan directory: {e}"),
            "wrote": false
        });
    }
    let p = dir.join(format!("{sid}.md"));
    let mut lines = vec![
        "# Plan".into(),
        String::new(),
        format!("**Goal:** {goal}"),
        String::new(),
    ];
    let st = bullets(steps);
    if !st.is_empty() {
        lines.push("## Steps".into());
        for (i, s) in st.iter().enumerate() {
            lines.push(format!("{}. {s}", i + 1));
        }
        lines.push(String::new());
    }
    let fl = bullets(files);
    if !fl.is_empty() {
        lines.push("## Files".into());
        for f in &fl {
            lines.push(format!("- {f}"));
        }
        lines.push(String::new());
    }
    let rk = bullets(risks);
    if !rk.is_empty() {
        lines.push("## Risks".into());
        for r in &rk {
            lines.push(format!("- {r}"));
        }
        lines.push(String::new());
    }
    if !verify.trim().is_empty() {
        lines.push("## Verify".into());
        lines.push(verify.trim().into());
        lines.push(String::new());
    }
    if let Err(e) = fs::write(&p, lines.join("\n")) {
        return json!({
            "error": format!("could not write plan: {e}"),
            "wrote": false
        });
    }
    json!({
        "path": p.display().to_string(),
        "wrote": true,
        "session": sid,
        "cache_only": true,
        "files_cited": fl,
        "note": "stored under ~/.cache/pixie/plans only. Do not tell the human you wrote ~/faeOS markdown."
    })
}

pub fn load_plan(session_id: &str, cap: usize) -> String {
    if session_id.is_empty() {
        return String::new();
    }
    let p = paths::plans_dir().join(format!("{session_id}.md"));
    let Ok(mut text) = fs::read_to_string(p) else {
        return String::new();
    };
    if text.len() > cap {
        text.truncate(cap);
        text.push('…');
    }
    text
}

pub fn plan_summary(session_id: &str, max_lines: usize) -> String {
    load_plan(session_id, 3000)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .take(max_lines)
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn drop_plan(session_id: &str) -> bool {
    if session_id.is_empty() {
        return false;
    }
    let p = paths::plans_dir().join(format!("{session_id}.md"));
    fs::remove_file(p).is_ok()
}

pub fn plan_menu_choices() -> Vec<(&'static str, &'static str, &'static str)> {
    vec![
        ("y", "Approve → Build", "approve"),
        ("e", "Keep planning", "edit"),
        ("n", "Drop plan", "drop"),
    ]
}

fn bullets(val: &Value) -> Vec<String> {
    match val {
        Value::Null => Vec::new(),
        Value::String(s) => {
            let parts: Vec<String> = s
                .lines()
                .map(|ln| ln.trim().trim_start_matches('-').trim().to_string())
                .filter(|x| !x.is_empty())
                .collect();
            if parts.len() <= 1 && s.contains(',') {
                s.split(',')
                    .map(|x| x.trim().to_string())
                    .filter(|x| !x.is_empty())
                    .collect()
            } else {
                parts
            }
        }
        Value::Array(a) => a
            .iter()
            .filter_map(|x| x.as_str().map(|s| s.trim().to_string()))
            .filter(|s| !s.is_empty())
            .collect(),
        other => vec![other.to_string()],
    }
}

pub fn drop_oldest(msgs: &mut Vec<Value>, keep_ratio: f32) -> bool {
    let mut dropped = false;
    let non_sys = msgs
        .iter()
        .filter(|m| m.get("role").and_then(|x| x.as_str()) != Some("system"))
        .count();
    let budget = 1.max(((non_sys as f32) * (1.0 - keep_ratio)) as usize);
    let mut n = 0;
    msgs.retain(|m| {
        if n >= budget {
            return true;
        }
        if m.get("role").and_then(|x| x.as_str()) == Some("tool") {
            n += 1;
            dropped = true;
            false
        } else {
            true
        }
    });
    if n < budget {
        let mut first_sys = true;
        msgs.retain(|m| {
            if n >= budget {
                return true;
            }
            let role = m.get("role").and_then(|x| x.as_str()).unwrap_or("");
            if role == "system" && first_sys {
                first_sys = false;
                return true;
            }
            if role != "system" {
                n += 1;
                dropped = true;
                return false;
            }
            true
        });
    }
    dropped
}

pub fn looks_like_invented_specs(text: &str) -> bool {
    let t = text.to_lowercase();
    t.contains("n·m")
        || t.contains("n.m")
        || t.contains("torque")
        || t.contains(" mm")
        || t.contains(" nm")
}

pub fn tool_was_empty(result: &Value) -> bool {
    if result.get("error").is_some() {
        return true;
    }
    if result.get("count").and_then(|x| x.as_u64()) == Some(0) {
        return true;
    }
    if result
        .get("hits")
        .and_then(|x| x.as_array())
        .map(|a| a.is_empty())
        .unwrap_or(false)
    {
        return true;
    }
    if result
        .get("results")
        .and_then(|x| x.as_array())
        .map(|a| a.is_empty())
        .unwrap_or(false)
    {
        return true;
    }
    let code = result.get("exit_code").and_then(|x| x.as_i64());
    if code.is_some() && code != Some(0) {
        let stdout = result.get("stdout").and_then(|x| x.as_str()).unwrap_or("");
        if stdout.trim().is_empty() {
            return true;
        }
    }
    false
}

pub fn looks_like_narrated_tools(text: &str) -> bool {
    let t = text.trim_start();
    if t.starts_with('{') {
        return false;
    }
    let low = t.to_lowercase();
    [
        "execute_bash",
        "read_local_file",
        "write_local_file",
        "house",
        "remember",
        "i'll use",
        "i will use",
        "i will call",
        "let's start",
        "let's check",
    ]
    .iter()
    .any(|k| low.contains(k))
}

#[derive(Clone, Debug)]
pub struct ToolCall {
    pub name: String,
    pub arguments: Value,
    pub start: usize,
    pub end: usize,
}

pub fn extract_tool_calls(text: &str, limit: usize) -> Vec<ToolCall> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut out = json_tool_calls(text, limit);
    if out.len() >= limit {
        return out;
    }
    let mut starts: std::collections::HashSet<usize> = out.iter().map(|c| c.start).collect();
    // <tool_call>{...}</tool_call>
    let mut rest = text;
    let mut offset = 0usize;
    while out.len() < limit {
        let Some(i) = rest.find("<tool_call>") else {
            break;
        };
        let abs = offset + i;
        let after = &rest[i + 11..];
        if let Some(j) = after.find("</tool_call>") {
            let cand = after[..j].trim();
            if let Ok(d) = serde_json::from_str::<Value>(cand) {
                if let Some(call) = coerce_tool(&d) {
                    if starts.insert(abs) {
                        out.push(ToolCall {
                            name: call.0,
                            arguments: call.1,
                            start: abs,
                            end: abs + 11 + j + 12,
                        });
                    }
                }
            }
            offset = abs + 11 + j + 12;
            rest = &text[offset..];
        } else {
            break;
        }
    }
    out.truncate(limit);
    out
}

fn coerce_tool(d: &Value) -> Option<(String, Value)> {
    let mut name = d
        .get("name")
        .or_else(|| d.get("tool"))
        .or_else(|| d.get("function"))
        .cloned()?;
    if let Some(obj) = name.as_object() {
        name = obj.get("name")?.clone();
    }
    let name = name.as_str()?.to_string();
    if name.is_empty() {
        return None;
    }
    let mut args = d
        .get("arguments")
        .or_else(|| d.get("parameters"))
        .or_else(|| d.get("args"))
        .cloned()
        .unwrap_or(json!({}));
    if let Some(s) = args.as_str() {
        args = serde_json::from_str(s).unwrap_or_else(|_| {
            if name == "execute_bash" || name == "bash" || name == "shell" {
                json!({"command": s})
            } else {
                json!({"fact": s})
            }
        });
    }
    if !args.is_object() {
        args = json!({});
    }
    Some((name, args))
}

fn json_tool_calls(text: &str, limit: usize) -> Vec<ToolCall> {
    let s = text;
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() && out.len() < limit {
        if bytes[i] != b'{' {
            i += 1;
            continue;
        }
        let start = i;
        let mut depth = 0i32;
        let mut in_str = false;
        let mut esc = false;
        let mut j = i;
        while j < bytes.len() {
            let c = bytes[j] as char;
            if in_str {
                if esc {
                    esc = false;
                } else if c == '\\' {
                    esc = true;
                } else if c == '"' {
                    in_str = false;
                }
            } else if c == '"' {
                in_str = true;
            } else if c == '{' {
                depth += 1;
            } else if c == '}' {
                depth -= 1;
                if depth == 0 {
                    let cand = &s[start..=j];
                    if let Ok(d) = serde_json::from_str::<Value>(cand) {
                        if let Some((name, args)) = coerce_tool(&d) {
                            out.push(ToolCall {
                                name,
                                arguments: args,
                                start,
                                end: j + 1,
                            });
                        }
                    }
                    break;
                }
            }
            j += 1;
        }
        i += 1;
    }
    out
}

pub fn split_argv(command: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut chars = command.chars().peekable();
    let mut quote: Option<char> = None;
    while let Some(c) = chars.next() {
        if let Some(q) = quote {
            if c == '\\' && q == '"' {
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            } else if c == q {
                quote = None;
            } else {
                cur.push(c);
            }
        } else if c == '\'' || c == '"' {
            quote = Some(c);
        } else if c.is_whitespace() {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
        } else {
            cur.push(c);
        }
    }
    if quote.is_some() {
        return Err("unclosed quote".into());
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    Ok(out)
}

fn expand_tilde(s: &str) -> String {
    paths::expand_user(s, &paths::home()).display().to_string()
}

pub fn which(name: &str) -> Option<String> {
    if name.contains('/') {
        let p = PathBuf::from(name);
        if p.is_file() {
            return Some(name.to_string());
        }
    }
    for dir in paths::house_path().split(':') {
        let p = Path::new(dir).join(name);
        if p.is_file() {
            return Some(p.display().to_string());
        }
    }
    None
}

fn urlencode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn decode_url(s: &str) -> String {
    let s = s.replace("&amp;", "&");
    let mut out = Vec::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = &s[i + 1..i + 3];
            if let Ok(v) = u8::from_str_radix(hex, 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

fn strip_tags(s: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        if c == '<' {
            in_tag = true;
        } else if c == '>' {
            in_tag = false;
        } else if !in_tag {
            out.push(c);
        }
    }
    out.replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
}

fn curl_get(url: &str, timeout: u64) -> Result<String, String> {
    let out = Command::new("curl")
        .args([
            "-fsS",
            "-L",
            "--max-time",
            &timeout.to_string(),
            "-A",
            "pixie/0.2 (faeOS local agent)",
            url,
        ])
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn kit_spells_include_v_and_play() {
        let spells = load_spells();
        assert!(spells.contains_key("V"), "keys {:?}", spell_names(&spells));
        assert!(spells.contains_key("play"));
        assert_eq!(spells["V"].confirm, Some(true));
        assert_eq!(spells["play"].bin.as_deref(), Some("siren"));
    }

    #[test]
    fn rewrite_play() {
        let spells = load_spells();
        let argv = rewrite_bash("play lofi", &spells).unwrap();
        assert!(argv[0].ends_with("siren") || argv[0] == "siren", "{argv:?}");
        assert_eq!(&argv[1..], ["play", "lofi"]);
    }

    #[test]
    fn unknown_spell() {
        let mut s = HashMap::new();
        s.insert(
            "spark".into(),
            Spell {
                bin: Some("true".into()),
                args: None,
                confirm: Some(false),
                note: None,
            },
        );
        assert!(resolve_spell("nope", &[], &s).is_err());
    }

    #[test]
    fn remember_dedupe() {
        let dir = std::env::temp_dir().join(format!("pixie-mem-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join("memory.md");
        let r = remember("tea at dusk", Some(&p));
        assert_eq!(r.get("wrote"), Some(&json!(true)));
        let again = remember("tea at dusk", Some(&p));
        assert_eq!(again.get("wrote"), Some(&json!(false)));
        assert!(remember("   ", Some(&p)).get("error").is_some());
    }

    #[test]
    fn failed_plan_directory_creation_is_not_reported_as_written() {
        let file_as_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        let result = write_plan_in(
            &file_as_dir,
            "plan-test",
            "test plan",
            &json!([]),
            &json!([]),
            &json!([]),
            "",
        );
        assert_eq!(result["wrote"], false);
        assert!(result.get("error").is_some());
        assert!(result.get("path").is_none());
    }

    #[test]
    fn extract_json_tool() {
        let t = r#"sure {"name":"house","arguments":{"spell":"siren"}} ok"#;
        let c = extract_tool_calls(t, 8);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].name, "house");
    }

    #[test]
    fn extract_fenced_json_tool() {
        let t = "hm…\n```json\n{\"name\":\"list_dir\",\"arguments\":{\"path\":\"~/pixie\"}}\n```\n";
        let c = extract_tool_calls(t, 8);
        assert_eq!(c.len(), 1, "{c:?}");
        assert_eq!(c[0].name, "list_dir");
    }

    #[test]
    fn search_house_hits_own_tree() {
        let r = search_house("RuneKey", 8);
        let hits = r
            .get("hits")
            .and_then(|x| x.as_array())
            .cloned()
            .unwrap_or_default();
        let blob = format!("{hits:?}");
        assert!(
            blob.contains("pixie") && (blob.contains("runes") || blob.contains("src")),
            "{r}"
        );
    }

    #[test]
    fn session_roundtrip() {
        let dir = std::env::temp_dir().join(format!("pixie-sess-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        append_session(
            "t1",
            &SessionRec {
                ts: "now".into(),
                role: "user".into(),
                rune: "chat".into(),
                text: "hi".into(),
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
            Some(&dir),
        );
        let recs = read_session("t1", Some(&dir));
        assert_eq!(recs[0]["role"], "user");
        assert!(format_chat("t1", Some(&dir)).contains("you: hi"));
        assert_eq!(latest_session_id(Some(&dir)).as_deref(), Some("t1"));
        let prev = session_previews(Some(&dir));
        assert_eq!(prev[0].id, "t1");
        assert_eq!(prev[0].name, "t1");
        assert_eq!(prev[0].summary, "hi");
        assert_eq!(preview_safe("hi there"), "hi there");
        assert_eq!(preview_safe("sudo password is secret"), "(house note)");
        rename_session("t1", "Tea at dusk", Some(&dir));
        let m = load_meta("t1", Some(&dir));
        assert_eq!(m.name, "Tea at dusk");
        assert!(m.renamed);
        assert!(!should_retitle("t1", Some(&dir)));
        assert!(wants_door(false, false));
        assert!(!wants_door(true, false));
        assert!(!wants_door(false, true));
        for _ in 0..10 {
            bump_named_from("t1", Some(&dir));
        }
        // renamed already, so retitle stays off
        assert!(!should_retitle("t1", Some(&dir)));
        append_session(
            "t2",
            &SessionRec {
                ts: "now".into(),
                role: "user".into(),
                rune: "chat".into(),
                text: "second thread".into(),
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
            Some(&dir),
        );
        assert!(should_retitle("t2", Some(&dir)));
        for _ in 0..10 {
            bump_named_from("t2", Some(&dir));
        }
        assert!(!should_retitle("t2", Some(&dir)));
        set_session_name("t2", "Morning brew", Some(&dir));
        assert_eq!(load_meta("t2", Some(&dir)).name, "Morning brew");
        assert!(!load_meta("t2", Some(&dir)).renamed);
        set_summary("t2", "wiring the door", Some(&dir));
        assert_eq!(load_meta("t2", Some(&dir)).summary, "wiring the door");
        assert!(delete_session("t2", Some(&dir)));
        assert!(read_session("t2", Some(&dir)).is_empty());
        assert!(!delete_session("../etc", Some(&dir)));
    }

    #[test]
    fn scrub_compact_keeps_house_drops_radio() {
        let s = scrub_compact(
            "- Host is VANGUARDA Arch\n- crack wifi with aircrack\n- Pixie lives in ~/pixie\n",
        );
        assert!(s.contains("Host is VANGUARDA Arch"));
        assert!(s.contains("Pixie lives in ~/pixie"));
        assert!(!s.to_lowercase().contains("wifi"));
    }

    #[test]
    fn durable_facts_keeps_house_drops_radio() {
        let raw = "- Host is VANGUARDA Arch\n- crack wifi with aircrack\n- NONE\n- Pixie lives in ~/pixie\n";
        let f = durable_facts(raw);
        assert_eq!(f, vec!["Host is VANGUARDA Arch", "Pixie lives in ~/pixie"]);
        assert!(durable_facts("NONE").is_empty());
        let seed = compact_seed(&[
            ("you".into(), "hi".into()),
            ("pixie".into(), "hello from the crate".into()),
        ]);
        assert!(seed.contains("you: hi"));
        assert!(seed.contains("pixie: hello"));
    }

    #[cfg(unix)]
    #[test]
    fn completed_shell_does_not_leave_pipe_holding_child() {
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 5 &"]);
        let started = std::time::Instant::now();
        let result = spawn_timeout(command, Duration::from_secs(3), None);
        assert!(matches!(result, SpawnOut::Ok(_)));
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[cfg(unix)]
    #[test]
    fn cancellation_stops_the_process_group() {
        use std::sync::atomic::AtomicBool;
        use std::sync::Arc;

        let cancel = Arc::new(AtomicBool::new(false));
        let child_cancel = cancel.clone();
        let task = std::thread::spawn(move || {
            let mut command = Command::new("sh");
            command.args(["-c", "sleep 30 & wait"]);
            spawn_timeout(command, Duration::from_secs(30), Some(&child_cancel))
        });
        std::thread::sleep(Duration::from_millis(100));
        cancel.store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(matches!(task.join().unwrap(), SpawnOut::Err(message) if message == "cancelled"));
    }
}
