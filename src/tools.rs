use crate::diff::{self, DiffLine, FileKind, FileTouch, Op};
use crate::mind;
use crate::paths;
use crate::runes::RuneKey;
use serde_json::{json, Map, Value};
use std::cell::RefCell;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

thread_local! {
    static TOUCH: RefCell<Option<FileTouch>> = const { RefCell::new(None) };
}

pub fn take_touch() -> Option<FileTouch> {
    TOUCH.with(|c| c.borrow_mut().take())
}

fn set_touch(t: FileTouch) {
    TOUCH.with(|c| *c.borrow_mut() = Some(t));
}

const TOOL_RESULT_CAP: usize = 1200;
const TOOL_ROUND_CAP: usize = 4000;

pub fn alias(name: &str) -> &str {
    match name {
        "create_file" | "create" | "write_file" | "write" | "file_create"
        | "mkfile" | "writefile" | "write_content" | "write_to_file" | "code_editor"
        | "text_editor" | "apply_patch" | "file_editor" => "write_local_file",
        "run_command" | "run" | "execute_command" | "shell" | "bash" | "run_bash"
        | "execute_script" | "run_terminal_command" | "shell_command" | "terminal"
        | "run_script" | "run_python" => "execute_bash",
        "read_file" | "read" => "read_local_file",
        "search_web" | "web_search" => "web_research",
        "fetch_url" | "open_url" | "read_page" => "fetch_web_page",
        "wikipedia" | "wiki_search" => "wiki",
        "grep_house" | "search_docs" => "search_house",
        "save_plan" | "plan" => "write_plan",
        "systemctl" => "manage_systemd_service",
        "check_email" | "mail" | "email" | "get_mail" | "list_mail" | "read_mail" | "inbox" => {
            "check_mail"
        }
        "cast" | "spell" | "run_spell" | "invoke" => "house",
        "memory" | "note" | "memorize" | "save_memory" => "remember",
        other => other,
    }
}

fn first_str<'a>(m: &'a Map<String, Value>, keys: &[&str]) -> String {
    for k in keys {
        if let Some(v) = m.get(*k) {
            if let Some(s) = v.as_str() {
                return s.to_string();
            }
            return v.to_string();
        }
    }
    String::new()
}

pub fn norm_args(fname: &str, fargs: &Value) -> Value {
    let m = match fargs.as_object() {
        Some(m) => m,
        None => return json!({}),
    };
    match fname {
        "write_local_file" => json!({
            "path": first_str(m, &["path", "file_path", "filename", "file", "filepath"]),
            "content": first_str(m, &["content", "file_text", "text", "new_string", "file_content"]),
        }),
        "execute_bash" => json!({
            "command": first_str(m, &["command", "cmd", "script", "command_string", "bash_command"]),
        }),
        "read_local_file" => json!({
            "path": first_str(m, &["path", "file_path", "filename", "filepath"]),
            "start_line": m.get("start_line"),
            "end_line": m.get("end_line"),
        }),
        "web_research" => json!({
            "query": first_str(m, &["query", "q", "search"]),
            "max_results": m.get("max_results").and_then(|x| x.as_i64()).unwrap_or(5),
        }),
        "check_mail" => {
            let b = first_str(m, &["box", "folder", "mailbox"]);
            json!({
                "sync": m.get("sync").or_else(|| m.get("sync_mail")).or_else(|| m.get("pull")).and_then(|x| x.as_bool()).unwrap_or(true),
                "box": if b.is_empty() { "unread".to_string() } else { b },
                "max_messages": m.get("max_messages").or_else(|| m.get("limit")).and_then(|x| x.as_i64()).unwrap_or(20),
            })
        }
        "house" => {
            let mut spell = first_str(m, &["spell", "name", "command", "bin"]);
            let mut args: Vec<String> = match m.get("args").or_else(|| m.get("arguments")).or_else(|| m.get("extra")) {
                Some(Value::Array(a)) => a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect(),
                Some(Value::String(s)) => mind::split_argv(s).unwrap_or_else(|_| vec![s.clone()]),
                _ => Vec::new(),
            };
            if spell.contains(' ') && args.is_empty() {
                if let Ok(parts) = mind::split_argv(&spell) {
                    if !parts.is_empty() {
                        spell = parts[0].clone();
                        args = parts[1..].to_vec();
                    }
                }
            }
            json!({"spell": spell, "args": args})
        }
        "remember" => json!({"fact": first_str(m, &["fact", "text", "note", "memory", "content"])}),
        "wiki" => json!({
            "query": first_str(m, &["query", "q", "search", "topic"]),
            "max_results": m.get("max_results").or_else(|| m.get("limit")).and_then(|x| x.as_i64()).unwrap_or(5),
        }),
        "search_house" => json!({"query": first_str(m, &["query", "q", "search", "pattern"])}),
        "write_plan" => json!({
            "goal": first_str(m, &["goal", "title"]),
            "steps": m.get("steps").or_else(|| m.get("plan")).cloned().unwrap_or(json!([])),
            "files": m.get("files").or_else(|| m.get("paths")).cloned().unwrap_or(json!([])),
            "risks": m.get("risks").or_else(|| m.get("tradeoffs")).cloned().unwrap_or(json!([])),
            "verify": first_str(m, &["verify", "check"]),
        }),
        "fetch_web_page" => json!({
            "url": first_str(m, &["url", "path"]),
            "max_chars": m.get("max_chars").and_then(|x| x.as_i64()).unwrap_or(6000),
        }),
        "manage_systemd_service" => {
            let a = first_str(m, &["action"]);
            json!({
                "service_name": first_str(m, &["service_name", "service", "name"]),
                "action": if a.is_empty() { "status".to_string() } else { a },
            })
        }
        _ => fargs.clone(),
    }
}

pub fn known_for(rune: RuneKey) -> &'static [&'static str] {
    match rune {
        RuneKey::Chat => &[
            "house",
            "remember",
            "search_house",
            "wiki",
            "execute_bash",
            "read_local_file",
            "manage_systemd_service",
            "check_mail",
            "look",
        ],
        RuneKey::Deep => &[
            "web_research",
            "fetch_web_page",
            "wiki",
            "search_house",
            "read_local_file",
            "look",
        ],
        RuneKey::Build => &[
            "house",
            "execute_bash",
            "read_local_file",
            "write_local_file",
            "look",
        ],
        RuneKey::Plan => &[
            "execute_bash",
            "read_local_file",
            "search_house",
            "wiki",
            "write_plan",
            "look",
        ],
    }
}

pub fn needs_confirm(fname: &str, rune: RuneKey) -> bool {
    match fname {
        "execute_bash" if rune != RuneKey::Plan => true,
        _ => false,
    }
}

fn plan_bash_ok(argv: &[String]) -> bool {
    let head = Path::new(&argv[0])
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(&argv[0]);
    match head {
        "ls" | "find" | "file" | "wc" | "head" | "tail" | "rg" | "grep"
        | "true" | "false" | "echo" | "whoami" | "pwd" | "uname" | "date" => true,
        "git" => {
            let sub = argv.get(1).map(|s| s.as_str()).unwrap_or("");
            matches!(sub, "status" | "diff" | "log" | "show" | "rev-parse")
        }
        _ => false,
    }
}

pub fn run_tool(
    fname: &str,
    fargs: &Value,
    rune: RuneKey,
    session_id: &str,
    confirm: Option<&dyn Fn(&str, &Value) -> bool>,
    cancel: Option<&AtomicBool>,
) -> Value {
    let fname = alias(fname).to_string();
    let allowed = known_for(rune);
    if fname == "look" {
        return json!({
            "error": "pixie: the eye is still sleeping",
            "next": "vision is Pass 2 — bind a VL model as pixie-eye"
        });
    }
    if !allowed.contains(&fname.as_str()) {
        return json!({
            "error": format!("Unknown tool '{fname}' in {} mode. Valid tools: {}", rune.as_str(), allowed.join(", "))
        });
    }
    let _ = take_touch();
    let fargs = norm_args(&fname, fargs);
    if needs_confirm(&fname, rune) {
        match confirm {
            None => {
                return json!({"error": format!("'{fname}' needs approval and none is available here; run `pixie` (interactive chat) to review and approve actions.")});
            }
            Some(c) if !c(&fname, &fargs) => {
                return json!({"error": format!("'{fname}' was not approved by the user.")});
            }
            _ => {}
        }
    }
    let spells = mind::load_spells();
    match fname.as_str() {
        "execute_bash" => {
            let cmd = fargs.get("command").and_then(|x| x.as_str()).unwrap_or("");
            if cmd.trim().is_empty() {
                return json!({"error": "empty command"});
            }
            let argv = match mind::rewrite_bash(cmd, &spells) {
                Ok(a) => a,
                Err(e) => return json!({"error": format!("could not parse command: {e}")}),
            };
            if argv.is_empty() {
                return json!({"error": "empty command"});
            }
            if rune == RuneKey::Plan && !plan_bash_ok(&argv) {
                return json!({"error": format!("Plan mode is read-only. '{}' is not on the allowlist (ls/find/file/wc/head/tail/rg/grep/git status|diff|log|show|rev-parse). Stay in Plan or switch to Build.", argv[0])});
            }
            let mut argv = argv;
            let head = Path::new(&argv[0]).file_name().and_then(|s| s.to_str()).unwrap_or("");
            if matches!(head, "systemctl" | "journalctl") && !argv.iter().any(|a| a == "--no-pager") {
                argv.insert(1, "--no-pager".into());
            }
            match cancel {
                Some(c) => mind::run_argv_cancel(&argv, Duration::from_secs(20), c),
                None => mind::run_argv(&argv, Duration::from_secs(20)),
            }
        }
        "read_local_file" => {
            let path = fargs.get("path").and_then(|x| x.as_str()).unwrap_or("");
            match paths::assert_allowed(path) {
                Err(e) => json!({"error": e.to_string()}),
                Ok(p) => {
                    match fs::read(&p) {
                        Err(e) => json!({"error": format!("File not found: {path} ({e})")}),
                        Ok(bytes) => {
                            if bytes.contains(&0) || std::str::from_utf8(&bytes).is_err() {
                                set_touch(FileTouch {
                                    path: p.display().to_string(),
                                    kind: FileKind::Read,
                                    added: 0,
                                    deleted: 0,
                                    lines: Vec::new(),
                                    binary: true,
                                    extra_paths: Vec::new(),
                                });
                                return json!({
                                    "path": p.display().to_string(),
                                    "error": "not a text file"
                                });
                            }
                            let text = String::from_utf8_lossy(&bytes);
                            let lines: Vec<&str> = text.lines().collect();
                            let start = fargs
                                .get("start_line")
                                .and_then(|x| x.as_i64())
                                .map(|n| (n as usize).saturating_sub(1).min(lines.len()))
                                .unwrap_or(0);
                            let end = fargs
                                .get("end_line")
                                .and_then(|x| x.as_i64())
                                .map(|n| (n as usize).min(lines.len()))
                                .unwrap_or(lines.len());
                            let slice = if start < end {
                                lines[start..end].join("\n")
                            } else {
                                lines[start..].join("\n")
                            };
                            let preview: Vec<DiffLine> = slice
                                .lines()
                                .take(80)
                                .map(|t| DiffLine {
                                    op: Op::Eq,
                                    text: t.to_string(),
                                })
                                .collect();
                            set_touch(FileTouch {
                                path: p.display().to_string(),
                                kind: FileKind::Read,
                                added: 0,
                                deleted: 0,
                                lines: preview,
                                binary: false,
                                extra_paths: Vec::new(),
                            });
                            json!({
                                "path": p.display().to_string(),
                                "content": slice,
                                "bounded_from": start + 1,
                                "bounded_to": end.max(start)
                            })
                        }
                    }
                }
            }
        }
        "write_local_file" => {
            let path = fargs.get("path").and_then(|x| x.as_str()).unwrap_or("");
            let content = fargs.get("content").and_then(|x| x.as_str()).unwrap_or("");
            match paths::assert_allowed(path) {
                Err(e) => json!({"error": e.to_string()}),
                Ok(p) => {
                    if let Some(d) = p.parent() {
                        let _ = fs::create_dir_all(d);
                    }
                    let existed = p.exists();
                    let old_bytes = if existed { fs::read(&p).ok() } else { None };
                    let binary_old = old_bytes
                        .as_ref()
                        .map(|b| b.contains(&0) || std::str::from_utf8(b).is_err())
                        .unwrap_or(false);
                    match fs::write(&p, content) {
                        Ok(()) => {
                            let created = !existed;
                            let (added, deleted, lines, binary) = if binary_old {
                                (0usize, 0usize, Vec::new(), true)
                            } else {
                                let old_text = old_bytes
                                    .as_ref()
                                    .and_then(|b| std::str::from_utf8(b).ok())
                                    .unwrap_or("");
                                let lines = diff::diff_text(old_text, content);
                                let (a, d) = diff::counts(&lines);
                                (a, d, lines, false)
                            };
                            set_touch(FileTouch {
                                path: p.display().to_string(),
                                kind: if created {
                                    FileKind::Create
                                } else {
                                    FileKind::Write
                                },
                                added,
                                deleted,
                                lines,
                                binary,
                                extra_paths: Vec::new(),
                            });
                            json!({
                                "path": p.display().to_string(),
                                "written": true,
                                "bytes": content.len(),
                                "created": created,
                                "added": added,
                                "deleted": deleted,
                                "binary": binary
                            })
                        }
                        Err(e) => json!({"error": e.to_string()}),
                    }
                }
            }
        }
        "house" => {
            let spell = fargs.get("spell").and_then(|x| x.as_str()).unwrap_or("");
            let extra: Vec<String> = fargs
                .get("args")
                .and_then(|x| x.as_array())
                .map(|a| a.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
                .unwrap_or_default();
            match mind::resolve_spell(spell, &extra, &spells) {
                Err(e) => json!({"error": e}),
                Ok(r) => {
                    if r.confirm {
                        match confirm {
                            None => {
                                return json!({"error": format!("'house {spell}' needs approval and none is available here; run `pixie` (interactive chat) to review and approve actions.")});
                            }
                            Some(c) if !c("house", &json!({"spell": spell, "args": extra})) => {
                                return json!({"error": format!("'house {spell}' was not approved by the user.")});
                            }
                            _ => {}
                        }
                    }
                    match cancel {
                        Some(c) => mind::run_argv_cancel(&r.argv, Duration::from_secs(20), c),
                        None => mind::run_argv(&r.argv, Duration::from_secs(20)),
                    }
                }
            }
        }
        "remember" => mind::remember(fargs.get("fact").and_then(|x| x.as_str()).unwrap_or(""), None),
        "search_house" => {
            let r = mind::search_house(fargs.get("query").and_then(|x| x.as_str()).unwrap_or(""), 12);
            let extra = hit_paths(&r);
            if let Some(first) = extra.first() {
                set_touch(FileTouch {
                    path: first.clone(),
                    kind: FileKind::Search,
                    added: 0,
                    deleted: 0,
                    lines: extra
                        .iter()
                        .take(12)
                        .map(|p| DiffLine {
                            op: Op::Eq,
                            text: p.clone(),
                        })
                        .collect(),
                    binary: false,
                    extra_paths: extra.clone(),
                });
            }
            r
        }
        "wiki" => {
            let r = mind::wiki(
                fargs.get("query").and_then(|x| x.as_str()).unwrap_or(""),
                fargs.get("max_results").and_then(|x| x.as_i64()).unwrap_or(5),
            );
            if !session_id.is_empty() {
                if let Some(arr) = r.get("results").and_then(|x| x.as_array()) {
                    for item in arr {
                        mind::note_source(
                            session_id,
                            item.get("url").and_then(|x| x.as_str()).unwrap_or(""),
                            item.get("title").and_then(|x| x.as_str()).unwrap_or(""),
                        );
                    }
                }
            }
            r
        }
        "web_research" => {
            let r = mind::search_web(
                fargs.get("query").and_then(|x| x.as_str()).unwrap_or(""),
                fargs.get("max_results").and_then(|x| x.as_i64()).unwrap_or(5),
            );
            if !session_id.is_empty() {
                if let Some(arr) = r.get("results").and_then(|x| x.as_array()) {
                    for item in arr {
                        mind::note_source(
                            session_id,
                            item.get("url").and_then(|x| x.as_str()).unwrap_or(""),
                            item.get("title").and_then(|x| x.as_str()).unwrap_or(""),
                        );
                    }
                }
            }
            r
        }
        "fetch_web_page" => {
            let r = mind::read_page(
                fargs.get("url").and_then(|x| x.as_str()).unwrap_or(""),
                fargs.get("max_chars").and_then(|x| x.as_i64()).unwrap_or(6000) as usize,
            );
            if !session_id.is_empty() {
                mind::note_source(
                    session_id,
                    r.get("url").and_then(|x| x.as_str()).unwrap_or(""),
                    r.get("title").and_then(|x| x.as_str()).unwrap_or(""),
                );
            }
            r
        }
        "write_plan" => {
            let r = mind::write_plan(
                fargs.get("goal").and_then(|x| x.as_str()).unwrap_or(""),
                fargs.get("steps").unwrap_or(&json!([])),
                fargs.get("files").unwrap_or(&json!([])),
                fargs.get("risks").unwrap_or(&json!([])),
                fargs.get("verify").and_then(|x| x.as_str()).unwrap_or(""),
                session_id,
            );
            if let Some(path) = r.get("path").and_then(|x| x.as_str()) {
                let preview = fs::read_to_string(path)
                    .unwrap_or_default()
                    .lines()
                    .take(80)
                    .map(|t| DiffLine {
                        op: Op::Add,
                        text: t.to_string(),
                    })
                    .collect::<Vec<_>>();
                let added = preview.len();
                set_touch(FileTouch {
                    path: path.to_string(),
                    kind: FileKind::Plan,
                    added,
                    deleted: 0,
                    lines: preview,
                    binary: false,
                    extra_paths: Vec::new(),
                });
            }
            r
        }
        "manage_systemd_service" => {
            let mut s_name = fargs.get("service_name").and_then(|x| x.as_str()).unwrap_or("menagerie.service");
            if matches!(s_name, "your_user_name" | "your_user_service" | "default" | "user-environment" | "") {
                s_name = "menagerie.service";
            }
            if !s_name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '@' | '.' | '_' | ':' | '-')) {
                return json!({"error": format!("invalid service name: {s_name:?}")});
            }
            let action = fargs.get("action").and_then(|x| x.as_str()).unwrap_or("status");
            if !matches!(action, "status" | "restart" | "logs") {
                return json!({"error": format!("Invalid action: {action}")});
            }
            let user = matches!(s_name, "menagerie.service" | "menagerie" | "kur");
            let mut cmd = vec!["systemctl".into()];
            if user {
                cmd.push("--user".into());
            }
            match action {
                "status" => {
                    cmd.push("status".into());
                    cmd.push(s_name.into());
                    cmd.push("--no-pager".into());
                }
                "restart" => {
                    cmd.push("restart".into());
                    cmd.push(s_name.into());
                }
                _ => {
                    cmd = vec![
                        "journalctl".into(),
                    ];
                    if user {
                        cmd.push("--user".into());
                    }
                    cmd.extend(["-u".into(), s_name.into(), "-n".into(), "15".into(), "--no-pager".into()]);
                }
            }
            let r = mind::run_argv(&cmd, Duration::from_secs(15));
            let mut stdout = r.get("stdout").and_then(|x| x.as_str()).unwrap_or("").to_string();
            if action == "status" && r.get("exit_code") == Some(&json!(0)) {
                stdout = stdout
                    .lines()
                    .filter(|line| {
                        ["Loaded:", "Active:", "Main PID:", "Tasks:", "Memory:"]
                            .iter()
                            .any(|k| line.contains(k))
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
            }
            json!({
                "service": s_name,
                "action": action,
                "stdout": stdout,
                "stderr": r.get("stderr").cloned().unwrap_or(json!("")),
            })
        }
        "check_mail" => check_mail(
            fargs.get("sync").and_then(|x| x.as_bool()).unwrap_or(true),
            fargs.get("box").and_then(|x| x.as_str()).unwrap_or("unread"),
            fargs.get("max_messages").and_then(|x| x.as_i64()).unwrap_or(20),
        ),
        _ => json!({"error": format!("Unknown function mapping: {fname}")}),
    }
}

fn check_mail(sync: bool, box_: &str, max_messages: i64) -> Value {
    let box_ = box_.trim().to_lowercase();
    if !matches!(box_.as_str(), "unread" | "read" | "trash") {
        return json!({"error": "box must be unread, read, or trash"});
    }
    let goblin = paths::home().join("bin/goblin");
    let run = |args: &[&str], _timeout: u64| -> String {
        if !goblin.is_file() {
            return format!("error: goblin not found at {}", goblin.display());
        }
        let mut cmd = Command::new(&goblin);
        cmd.args(args);
        cmd.env("PATH", paths::house_path());
        match cmd.output() {
            Err(e) => format!("error: could not run goblin: {e}"),
            Ok(o) => {
                let mut parts = Vec::new();
                let out = String::from_utf8_lossy(&o.stdout).trim().to_string();
                let err = String::from_utf8_lossy(&o.stderr).trim().to_string();
                if !out.is_empty() {
                    parts.push(out);
                }
                if !err.is_empty() {
                    parts.push(format!("[log]\n{err}"));
                }
                if !o.status.success() {
                    parts.push(format!("(exit {})", o.status.code().unwrap_or(-1)));
                }
                if parts.is_empty() {
                    "(no output)".into()
                } else {
                    parts.join("\n")
                }
            }
        }
    };
    let mut result = json!({"box": box_});
    if sync {
        result["sync"] = json!(run(&["sync"], 180));
    }
    let listing = run(&["list", &box_, "--plain"], 60);
    result["listing"] = json!(listing);
    let mut subjects = Vec::new();
    for line in listing.lines() {
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() >= 4 && fields[3].starts_with("subject=") {
            subjects.push(fields[3][8..].to_string());
        }
    }
    result["count"] = json!(subjects.len());
    result["subjects"] = json!(subjects);
    if box_ == "unread" {
        let limit = max_messages.clamp(1, 50);
        result["digest"] = json!(run(&["bundle", "--limit", &limit.to_string()], 60));
    } else {
        result["digest"] = json!("(no digest for read/trash — use the listing)");
    }
    result
}

fn hit_paths(result: &Value) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(arr) = result.get("hits").and_then(|x| x.as_array()) {
        for h in arr {
            if let Some(p) = h.get("path").and_then(|x| x.as_str()) {
                if !out.iter().any(|e: &String| e == p) {
                    out.push(p.to_string());
                }
            }
        }
    }
    out
}

/// Resume a file card from jsonl without storing the hunk (look re-reads the path).
pub fn touch_from_rec(name: &str, args: &Value, result: &Value) -> Option<FileTouch> {
    if result.get("error").is_some() {
        return None;
    }
    match alias(name) {
        "read_local_file" => {
            let path = result
                .get("path")
                .or_else(|| args.get("path"))
                .and_then(|x| x.as_str())?;
            Some(FileTouch {
                path: path.into(),
                kind: FileKind::Read,
                added: 0,
                deleted: 0,
                lines: Vec::new(),
                binary: false,
                extra_paths: Vec::new(),
            })
        }
        "write_local_file" => {
            let path = result.get("path").and_then(|x| x.as_str())?;
            let created = result.get("created").and_then(|x| x.as_bool()).unwrap_or(false);
            Some(FileTouch {
                path: path.into(),
                kind: if created {
                    FileKind::Create
                } else {
                    FileKind::Write
                },
                added: result.get("added").and_then(|x| x.as_u64()).unwrap_or(0) as usize,
                deleted: result.get("deleted").and_then(|x| x.as_u64()).unwrap_or(0) as usize,
                lines: Vec::new(),
                binary: result.get("binary").and_then(|x| x.as_bool()).unwrap_or(false),
                extra_paths: Vec::new(),
            })
        }
        "search_house" => {
            let extra = hit_paths(result);
            let path = extra.first()?.clone();
            Some(FileTouch {
                path,
                kind: FileKind::Search,
                added: 0,
                deleted: 0,
                lines: Vec::new(),
                binary: false,
                extra_paths: extra,
            })
        }
        "write_plan" => {
            let path = result.get("path").and_then(|x| x.as_str())?;
            Some(FileTouch {
                path: path.into(),
                kind: FileKind::Plan,
                added: 0,
                deleted: 0,
                lines: Vec::new(),
                binary: false,
                extra_paths: Vec::new(),
            })
        }
        _ => None,
    }
}

pub fn tool_payload(results: &[Value]) -> String {
    let mut parts = Vec::new();
    let mut used = 0usize;
    for item in results {
        let mut s = item.to_string();
        if s.len() > TOOL_RESULT_CAP {
            let n = s.len();
            s.truncate(TOOL_RESULT_CAP);
            s.push_str(&format!("… [truncated: was {n} chars]"));
        }
        if used + s.len() > TOOL_ROUND_CAP {
            break;
        }
        used += s.len();
        parts.push(s);
    }
    format!("[{}]", parts.join(", "))
}

pub fn tool_body(fname: &str, fargs: &Value, result: &Value, verbose: bool) -> String {
    let routing = format!("Routing: {fname} -> {fargs}");
    if !verbose {
        if let Some(e) = result.get("error") {
            return format!("{routing}\nerror: {}", e.to_string().chars().take(180).collect::<String>());
        }
        if let Some(s) = result.get("stdout").and_then(|x| x.as_str()) {
            return format!("{routing}\n{}", s.chars().take(120).collect::<String>());
        }
        if result.get("written").and_then(|x| x.as_bool()).unwrap_or(false) {
            let bytes = result.get("bytes").and_then(|x| x.as_u64()).unwrap_or(0);
            let a = result.get("added").and_then(|x| x.as_u64()).unwrap_or(0);
            let d = result.get("deleted").and_then(|x| x.as_u64()).unwrap_or(0);
            return format!("{routing}\nwrote {bytes} bytes (+{a} -{d})");
        }
        if let Some(c) = result.get("count") {
            return format!("{routing}\n{c} results");
        }
        return routing;
    }
    format!("{routing}\n{result}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aliases() {
        assert_eq!(alias("cast"), "house");
        assert_eq!(alias("memory"), "remember");
        assert_eq!(alias("code_editor"), "write_local_file");
    }

    #[test]
    fn house_splits_spell_string() {
        let got = norm_args("house", &json!({"spell": "siren play lofi"}));
        assert_eq!(got["spell"], "siren");
        assert_eq!(got["args"], json!(["play", "lofi"]));
    }

    #[test]
    fn plan_refuses_rm() {
        let r = run_tool(
            "execute_bash",
            &json!({"command": "rm -rf /tmp/nope"}),
            RuneKey::Plan,
            "",
            None,
            None,
        );
        assert!(r.get("error").is_some(), "{r}");
        let msg = r["error"].as_str().unwrap();
        assert!(msg.contains("read-only") || msg.contains("allowlist"), "{msg}");
        // mutating git is also refused
        let r2 = run_tool(
            "execute_bash",
            &json!({"command": "git commit -m x"}),
            RuneKey::Plan,
            "",
            None,
            None,
        );
        assert!(r2.get("error").is_some(), "{r2}");
    }

    #[test]
    fn chat_rejects_web_research() {
        let r = run_tool(
            "web_research",
            &json!({"query": "hi"}),
            RuneKey::Chat,
            "",
            None,
            None,
        );
        let msg = r["error"].as_str().unwrap_or("");
        assert!(msg.contains("Unknown tool") || msg.contains("web_research"), "{r}");
    }

    #[test]
    fn look_stub() {
        let r = run_tool("look", &json!({"target": "screen"}), RuneKey::Chat, "", None, None);
        assert!(r["error"].as_str().unwrap().contains("sleeping"));
    }

    #[test]
    fn write_reports_diff_not_hunks() {
        let dir = paths::cache_dir().join(format!("tui-feel-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let p = dir.join("sample.txt");
        let path = p.display().to_string();
        let r = run_tool(
            "write_local_file",
            &json!({"path": path, "content": "alpha\nbeta\n"}),
            RuneKey::Build,
            "",
            None,
            None,
        );
        assert_eq!(r.get("written"), Some(&json!(true)), "{r}");
        assert_eq!(r.get("created"), Some(&json!(true)), "{r}");
        assert!(r.get("added").and_then(|x| x.as_u64()).unwrap_or(0) >= 2, "{r}");
        assert!(r.get("hunks").is_none(), "model payload must not dump hunks: {r}");
        let blob = r.to_string();
        assert!(!blob.contains("hunks"), "{blob}");
        let touch = take_touch().expect("file touch");
        assert!(touch.lines.iter().any(|l| l.op == Op::Add));

        let r2 = run_tool(
            "write_local_file",
            &json!({"path": path, "content": "alpha\ngamma\n"}),
            RuneKey::Build,
            "",
            None,
            None,
        );
        assert_eq!(r2.get("created"), Some(&json!(false)), "{r2}");
        assert!(r2.get("deleted").and_then(|x| x.as_u64()).unwrap_or(0) >= 1, "{r2}");
        let t2 = take_touch().expect("edit touch");
        assert!(t2.lines.iter().any(|l| l.op == Op::Del && l.text.contains("beta")));
        assert!(t2.lines.iter().any(|l| l.op == Op::Add && l.text.contains("gamma")));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn bash_true() {
        let r = run_tool(
            "execute_bash",
            &json!({"command": "true"}),
            RuneKey::Plan,
            "",
            None,
            None,
        );
        assert_eq!(r.get("exit_code"), Some(&json!(0)), "{r}");
    }

    #[test]
    fn oneshot_denies_confirm_bash() {
        let r = run_tool(
            "execute_bash",
            &json!({"command": "true"}),
            RuneKey::Chat,
            "",
            None,
            None,
        );
        assert!(r.get("error").is_some(), "{r}");
    }
}
