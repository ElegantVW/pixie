use crate::attach::{self, Kind};
use crate::diff::{self, DiffLine, FileKind, FileTouch, Op};
use crate::kitty;
use crate::llm;
use crate::mind;
use crate::paths;
use crate::prefs;
use crate::runes::RuneKey;
use serde_json::{json, Map, Value};
use std::cell::RefCell;
use std::fs;
use std::io::Cursor;
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
        "create_file" | "create" | "write_file" | "write" | "file_create" | "mkfile"
        | "writefile" | "write_content" | "write_to_file" | "code_editor" => "write_local_file",
        "apply_patch" | "text_editor" | "file_editor" | "str_replace" | "replace_in_file"
        | "edit_file" | "edit" => "edit_local_file",
        "ls" | "listdir" | "list_directory" | "list" => "list_dir",
        "grep" | "ripgrep" | "rg_files" | "search_files" => "grep_files",
        "run_command"
        | "run"
        | "execute_command"
        | "shell"
        | "bash"
        | "run_bash"
        | "execute_script"
        | "run_terminal_command"
        | "shell_command"
        | "terminal"
        | "run_script"
        | "run_python" => "execute_bash",
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
        "whatsapp" | "wa" | "wacli" | "check_wa" => "check_whatsapp",
        "imagine" | "draw" | "imp_paint" | "generate_art" => "generate_image",
        "see" | "describe" | "vision" | "eye" => "look",
        "cast" | "spell" | "run_spell" | "invoke" => "house",
        "memory" | "note" | "memorize" | "save_memory" => "remember",
        other => other,
    }
}

fn first_str(m: &Map<String, Value>, keys: &[&str]) -> String {
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
            "content": first_str(m, &["content", "file_text", "text", "file_content"]),
        }),
        "edit_local_file" => json!({
            "path": first_str(m, &["path", "file_path", "filename", "file", "filepath"]),
            "old_string": first_str(m, &["old_string", "old", "search", "find"]),
            "new_string": first_str(m, &["new_string", "new", "replace", "content"]),
        }),
        "list_dir" => json!({
            "path": first_str(m, &["path", "dir", "directory", "folder"]),
            "max": m.get("max").or_else(|| m.get("limit")).cloned().unwrap_or(json!(200)),
        }),
        "grep_files" => json!({
            "path": first_str(m, &["path", "dir", "root"]),
            "pattern": first_str(m, &["pattern", "query", "q", "search", "needle"]),
            "glob": first_str(m, &["glob", "include", "filter"]),
            "max": m.get("max").or_else(|| m.get("limit")).cloned().unwrap_or(json!(40)),
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
            let a = first_str(m, &["action", "op", "cmd"]);
            json!({
                "action": if a.is_empty() { "list".to_string() } else { a },
                "sync": m.get("sync").or_else(|| m.get("sync_mail")).or_else(|| m.get("pull")).and_then(|x| x.as_bool()).unwrap_or(false),
                "box": if b.is_empty() { "unread".to_string() } else { b },
                "query": first_str(m, &["query", "q", "search"]),
                "id": first_str(m, &["id", "file", "uid", "name"]),
                "to": first_str(m, &["to", "recipient"]),
                "subject": first_str(m, &["subject", "title"]),
                "body": first_str(m, &["body", "text", "message"]),
                "max_messages": m.get("max_messages").or_else(|| m.get("limit")).and_then(|x| x.as_i64()).unwrap_or(20),
            })
        }
        "check_whatsapp" => {
            let a = first_str(m, &["action", "op", "cmd"]);
            let q = first_str(m, &["query", "q", "search"]);
            json!({
                "action": if a.is_empty() {
                    if q.is_empty() { "list".to_string() } else { "search".to_string() }
                } else { a },
                "query": q,
                "chat": first_str(m, &["chat", "jid", "name"]),
                "id": first_str(m, &["id", "msgid", "message_id"]),
                "to": first_str(m, &["to", "recipient"]),
                "text": first_str(m, &["text", "message", "body", "caption"]),
                "caption": first_str(m, &["caption", "text", "message", "body"]),
                "path": first_str(m, &["path", "file", "filepath", "file_path"]),
                "file": first_str(m, &["file", "path", "filepath", "file_path"]),
                "after": first_str(m, &["after", "since"]),
                "before": first_str(m, &["before", "until"]),
                "limit": m.get("limit").or_else(|| m.get("max")).and_then(|x| x.as_i64()).unwrap_or(20),
            })
        }
        "look" => json!({
            "path": first_str(m, &["path", "file", "image", "target", "file_path", "filepath"]),
            "query": first_str(m, &["query", "question", "prompt", "q"]),
        }),
        "generate_image" => json!({
            "wish": first_str(m, &["wish", "prompt", "text", "query"]),
        }),
        "house" => {
            let mut spell = first_str(m, &["spell", "name", "command", "bin"]);
            let mut args: Vec<String> = match m
                .get("args")
                .or_else(|| m.get("arguments"))
                .or_else(|| m.get("extra"))
            {
                Some(Value::Array(a)) => a
                    .iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .collect(),
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
            "list_dir",
            "grep_files",
            "execute_bash",
            "read_local_file",
            "manage_systemd_service",
            "check_mail",
            "check_whatsapp",
            "generate_image",
            "look",
        ],
        RuneKey::Deep => &[
            "web_research",
            "fetch_web_page",
            "wiki",
            "search_house",
            "read_local_file",
            "list_dir",
            "grep_files",
            "look",
        ],
        RuneKey::Build => &[
            "house",
            "search_house",
            "list_dir",
            "grep_files",
            "execute_bash",
            "read_local_file",
            "edit_local_file",
            "write_local_file",
            "generate_image",
            "look",
        ],
        RuneKey::Plan => &[
            "execute_bash",
            "read_local_file",
            "list_dir",
            "grep_files",
            "search_house",
            "wiki",
            "write_plan",
            "look",
        ],
    }
}

#[allow(dead_code)]
pub fn needs_confirm(fname: &str, rune: RuneKey) -> bool {
    needs_confirm_args(fname, rune, &json!({}))
}

pub fn needs_confirm_args(fname: &str, rune: RuneKey, fargs: &Value) -> bool {
    match fname {
        "execute_bash" if rune != RuneKey::Plan => true,
        "generate_image" => true,
        "check_mail" => {
            let a = fargs
                .get("action")
                .and_then(|x| x.as_str())
                .unwrap_or("list");
            matches!(a, "sync" | "steal" | "keep" | "trash" | "send")
                || fargs.get("sync").and_then(|x| x.as_bool()).unwrap_or(false)
        }
        "check_whatsapp" => {
            let a = fargs
                .get("action")
                .and_then(|x| x.as_str())
                .unwrap_or("list");
            matches!(a, "send" | "send_file" | "send_voice" | "sync" | "mark_read")
        }
        _ => false,
    }
}

fn plan_bash_ok(argv: &[String]) -> bool {
    let head = Path::new(&argv[0])
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(&argv[0]);
    match head {
        "ls" | "find" | "file" | "wc" | "head" | "tail" | "rg" | "grep" | "true" | "false"
        | "echo" | "whoami" | "pwd" | "uname" | "date" => true,
        "git" => {
            let sub = argv.get(1).map(|s| s.as_str()).unwrap_or("");
            matches!(sub, "status" | "diff" | "log" | "show" | "rev-parse")
        }
        _ => false,
    }
}

fn list_dir_json(p: &Path, max: usize) -> Value {
    if p.is_file() {
        return json!({"path": p.display().to_string(), "entries": [{"name": p.file_name().and_then(|s| s.to_str()).unwrap_or(""), "kind": "file"}], "count": 1});
    }
    let Ok(rd) = fs::read_dir(p) else {
        return json!({"error": format!("not a directory: {}", p.display())});
    };
    let mut names: Vec<(String, &'static str)> = rd
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let kind = if e.path().is_dir() { "dir" } else { "file" };
            Some((name, kind))
        })
        .collect();
    names.sort_by(|a, b| a.0.cmp(&b.0));
    let count = names.len();
    let entries: Vec<Value> = names
        .into_iter()
        .take(max.max(1))
        .map(|(name, kind)| json!({"name": name, "kind": kind}))
        .collect();
    json!({"path": p.display().to_string(), "entries": entries, "count": count})
}

fn glob_match(pat: &str, name: &str) -> bool {
    if pat.is_empty() || pat == "*" {
        return true;
    }
    glob_rec(pat.as_bytes(), name.as_bytes())
}

fn glob_rec(p: &[u8], s: &[u8]) -> bool {
    match (p.first().copied(), s.first().copied()) {
        (None, None) => true,
        (Some(b'*'), _) => glob_rec(&p[1..], s) || (!s.is_empty() && glob_rec(p, &s[1..])),
        (Some(b'?'), Some(_)) => glob_rec(&p[1..], &s[1..]),
        (Some(a), Some(b)) if a.eq_ignore_ascii_case(&b) => glob_rec(&p[1..], &s[1..]),
        _ => false,
    }
}

fn skip_walk_dir(name: &str) -> bool {
    matches!(
        name,
        ".git" | "target" | "node_modules" | "__pycache__" | ".cargo" | ".cache"
    )
}

fn grep_walk(
    root: &Path,
    pattern: &str,
    glob: &str,
    max_hits: usize,
    max_files: usize,
    hits: &mut Vec<Value>,
    extra: &mut Vec<String>,
) {
    let mut files = Vec::new();
    if root.is_file() {
        files.push(root.to_path_buf());
    } else {
        collect_files(root, glob, max_files, &mut files);
    }
    let needle = pattern.to_lowercase();
    for p in files {
        if hits.len() >= max_hits {
            break;
        }
        let Ok(bytes) = fs::read(&p) else { continue };
        if bytes.contains(&0) || std::str::from_utf8(&bytes).is_err() {
            continue;
        }
        let text = String::from_utf8_lossy(&bytes);
        for (i, line) in text.lines().enumerate() {
            if line.to_lowercase().contains(&needle) {
                let path = p.display().to_string();
                if !extra.iter().any(|e| e == &path) {
                    extra.push(path.clone());
                }
                hits.push(json!({
                    "path": path,
                    "line": i + 1,
                    "text": line.chars().take(200).collect::<String>()
                }));
                if hits.len() >= max_hits {
                    break;
                }
            }
        }
    }
}

fn collect_files(root: &Path, glob: &str, max_files: usize, out: &mut Vec<std::path::PathBuf>) {
    if out.len() >= max_files {
        return;
    }
    let Ok(rd) = fs::read_dir(root) else { return };
    for e in rd.flatten() {
        if out.len() >= max_files {
            return;
        }
        let p = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if p.is_dir() {
            if skip_walk_dir(&name) || name.starts_with('.') {
                continue;
            }
            collect_files(&p, glob, max_files, out);
        } else if glob_match(glob, &name) {
            out.push(p);
        }
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
    if !allowed.contains(&fname.as_str()) {
        return json!({
            "error": format!("Unknown tool '{fname}' in {} mode. Valid tools: {}", rune.as_str(), allowed.join(", "))
        });
    }
    let _ = take_touch();
    let fargs = norm_args(&fname, fargs);
    if needs_confirm_args(&fname, rune, &fargs) {
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
            let mut argv = if rune != RuneKey::Plan && mind::needs_shell(cmd) {
                vec!["sh".into(), "-c".into(), cmd.to_string()]
            } else {
                argv
            };
            let head = Path::new(&argv[0])
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("");
            if matches!(head, "systemctl" | "journalctl") && !argv.iter().any(|a| a == "--no-pager")
            {
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
                Ok(p) => match fs::read(&p) {
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
                                collapsed: false,
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
                            collapsed: false,
                        });
                        json!({
                            "path": p.display().to_string(),
                            "content": slice,
                            "bounded_from": start + 1,
                            "bounded_to": end.max(start)
                        })
                    }
                },
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
                                collapsed: false,
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
        "edit_local_file" => {
            let path = fargs.get("path").and_then(|x| x.as_str()).unwrap_or("");
            let old = fargs
                .get("old_string")
                .and_then(|x| x.as_str())
                .unwrap_or("");
            let new = fargs
                .get("new_string")
                .and_then(|x| x.as_str())
                .unwrap_or("");
            if old.is_empty() {
                json!({"error": "empty old_string", "next": "read the file and pass the exact text to replace"})
            } else {
                match paths::assert_allowed(path) {
                    Err(e) => json!({"error": e.to_string()}),
                    Ok(p) => match fs::read_to_string(&p) {
                        Err(e) => {
                            json!({"error": format!("File not found: {path} ({e})"), "next": "list_dir or check the path"})
                        }
                        Ok(text) => {
                            let n = text.match_indices(old).count();
                            if n == 0 {
                                json!({"error": "old_string not found", "next": "read the file and copy the exact text to replace"})
                            } else if n > 1 {
                                json!({"error": format!("old_string appears {n} times"), "next": "read the file and pass a unique old_string"})
                            } else {
                                let content = text.replacen(old, new, 1);
                                match fs::write(&p, &content) {
                                    Err(e) => json!({"error": e.to_string()}),
                                    Ok(()) => {
                                        let lines = diff::diff_text(&text, &content);
                                        let (added, deleted) = diff::counts(&lines);
                                        set_touch(FileTouch {
                                            path: p.display().to_string(),
                                            kind: FileKind::Write,
                                            added,
                                            deleted,
                                            lines,
                                            binary: false,
                                            extra_paths: Vec::new(),
                                            collapsed: false,
                                        });
                                        json!({
                                            "path": p.display().to_string(),
                                            "edited": true,
                                            "added": added,
                                            "deleted": deleted
                                        })
                                    }
                                }
                            }
                        }
                    },
                }
            }
        }
        "list_dir" => {
            let path = fargs.get("path").and_then(|x| x.as_str()).unwrap_or("");
            let path = if path.is_empty() { "." } else { path };
            let max = fargs.get("max").and_then(|x| x.as_u64()).unwrap_or(200) as usize;
            match paths::assert_allowed(path) {
                Err(e) => json!({"error": e.to_string()}),
                Ok(p) => list_dir_json(&p, max),
            }
        }
        "grep_files" => {
            let path = fargs.get("path").and_then(|x| x.as_str()).unwrap_or("");
            let path = if path.is_empty() { "." } else { path };
            let pattern = fargs.get("pattern").and_then(|x| x.as_str()).unwrap_or("");
            let glob = fargs.get("glob").and_then(|x| x.as_str()).unwrap_or("");
            let max = fargs.get("max").and_then(|x| x.as_u64()).unwrap_or(40) as usize;
            if pattern.trim().is_empty() {
                json!({"error": "empty pattern"})
            } else {
                match paths::assert_allowed(path) {
                    Err(e) => json!({"error": e.to_string()}),
                    Ok(p) => {
                        let mut hits = Vec::new();
                        let mut extra = Vec::new();
                        grep_walk(&p, pattern, glob, max, 400, &mut hits, &mut extra);
                        if let Some(first) = extra.first() {
                            set_touch(FileTouch {
                                path: first.clone(),
                                kind: FileKind::Search,
                                added: 0,
                                deleted: 0,
                                lines: extra
                                    .iter()
                                    .take(12)
                                    .map(|s| DiffLine {
                                        op: Op::Eq,
                                        text: s.clone(),
                                    })
                                    .collect(),
                                binary: false,
                                extra_paths: extra.clone(),
                                collapsed: false,
                            });
                        }
                        json!({"path": p.display().to_string(), "pattern": pattern, "hits": hits, "count": hits.len()})
                    }
                }
            }
        }
        "house" => {
            let spell = fargs.get("spell").and_then(|x| x.as_str()).unwrap_or("");
            let extra: Vec<String> = fargs
                .get("args")
                .and_then(|x| x.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect()
                })
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
        "remember" => mind::remember(
            fargs.get("fact").and_then(|x| x.as_str()).unwrap_or(""),
            None,
        ),
        "search_house" => {
            let r = mind::search_house(
                fargs.get("query").and_then(|x| x.as_str()).unwrap_or(""),
                12,
            );
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
                    collapsed: false,
                });
            }
            r
        }
        "wiki" => {
            let r = mind::wiki(
                fargs.get("query").and_then(|x| x.as_str()).unwrap_or(""),
                fargs
                    .get("max_results")
                    .and_then(|x| x.as_i64())
                    .unwrap_or(5),
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
                fargs
                    .get("max_results")
                    .and_then(|x| x.as_i64())
                    .unwrap_or(5),
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
                fargs
                    .get("max_chars")
                    .and_then(|x| x.as_i64())
                    .unwrap_or(6000) as usize,
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
                    collapsed: false,
                });
            }
            r
        }
        "manage_systemd_service" => {
            let mut s_name = fargs
                .get("service_name")
                .and_then(|x| x.as_str())
                .unwrap_or("menagerie.service");
            if matches!(
                s_name,
                "your_user_name" | "your_user_service" | "default" | "user-environment" | ""
            ) {
                s_name = "menagerie.service";
            }
            if !s_name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '@' | '.' | '_' | ':' | '-'))
            {
                return json!({"error": format!("invalid service name: {s_name:?}")});
            }
            let action = fargs
                .get("action")
                .and_then(|x| x.as_str())
                .unwrap_or("status");
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
                    cmd = vec!["journalctl".into()];
                    if user {
                        cmd.push("--user".into());
                    }
                    cmd.extend([
                        "-u".into(),
                        s_name.into(),
                        "-n".into(),
                        "15".into(),
                        "--no-pager".into(),
                    ]);
                }
            }
            let r = mind::run_argv(&cmd, Duration::from_secs(15));
            let mut stdout = r
                .get("stdout")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
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
        "check_mail" => check_mail(&fargs),
        "check_whatsapp" => check_whatsapp(&fargs),
        "generate_image" => generate_image(
            fargs.get("wish").and_then(|x| x.as_str()).unwrap_or(""),
            cancel,
            confirm,
        ),
        "look" => look(
            fargs.get("path").and_then(|x| x.as_str()).unwrap_or(""),
            fargs.get("query").and_then(|x| x.as_str()).unwrap_or(""),
            cancel,
        ),
        _ => json!({"error": format!("Unknown function mapping: {fname}")}),
    }
}

fn goblin_bin() -> std::path::PathBuf {
    let p = paths::home().join("bin/goblin");
    if p.is_file() {
        p
    } else {
        std::path::PathBuf::from("goblin")
    }
}

fn run_cmd(bin: &Path, args: &[&str], timeout: Duration) -> Value {
    let mut argv = vec![bin.display().to_string()];
    argv.extend(args.iter().map(|s| s.to_string()));
    mind::run_argv(&argv, timeout)
}

fn cmd_text(v: &Value) -> String {
    let out = v.get("stdout").and_then(|x| x.as_str()).unwrap_or("");
    let err = v.get("stderr").and_then(|x| x.as_str()).unwrap_or("");
    let code = v.get("exit_code").and_then(|x| x.as_i64()).unwrap_or(0);
    let mut s = out.trim().to_string();
    if !err.trim().is_empty() {
        if !s.is_empty() {
            s.push('\n');
        }
        s.push_str(err.trim());
    }
    if code != 0 {
        if !s.is_empty() {
            s.push('\n');
        }
        s.push_str(&format!("(exit {code})"));
    }
    if s.is_empty() {
        "(no output)".into()
    } else {
        s
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn parse_to_unix(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let o = Command::new("date").args(["-d", s, "+%s"]).output().ok()?;
    if !o.status.success() {
        return None;
    }
    String::from_utf8_lossy(&o.stdout).trim().parse().ok()
}

fn age_phrase(date: &str) -> String {
    let Some(t) = parse_to_unix(date) else {
        return "?".into();
    };
    let d = unix_now().saturating_sub(t);
    if d < 90 {
        format!("{d}s")
    } else if d < 3600 {
        format!("{}m", d / 60)
    } else if d < 86400 {
        format!("{}h", d / 3600)
    } else {
        format!("{}d", d / 86400)
    }
}

fn kv_line(line: &str) -> std::collections::HashMap<String, String> {
    let fields: Vec<&str> = line.split('\t').collect();
    let mut m = std::collections::HashMap::new();
    let mut i = 0;
    if fields.first().is_some_and(|s| !s.contains('=')) {
        if fields.get(1).is_some_and(|s| !s.contains('=')) {
            m.insert("box".into(), fields[0].to_string());
            m.insert("id".into(), fields[1].to_string());
            i = 2;
        } else {
            m.insert("id".into(), fields[0].to_string());
            i = 1;
        }
    }
    for p in fields.iter().skip(i) {
        if let Some((k, v)) = p.split_once('=') {
            m.insert(k.to_string(), v.to_string());
        }
    }
    m
}

fn check_mail(fargs: &Value) -> Value {
    let mut action = fargs
        .get("action")
        .and_then(|x| x.as_str())
        .unwrap_or("list")
        .trim()
        .to_lowercase();
    if fargs.get("sync").and_then(|x| x.as_bool()).unwrap_or(false) && action == "list" {
        action = "sync".into();
    }
    let box_ = fargs
        .get("box")
        .and_then(|x| x.as_str())
        .unwrap_or("unread")
        .trim()
        .to_lowercase();
    if !matches!(box_.as_str(), "unread" | "read" | "trash") {
        return json!({"error": "box must be unread, read, or trash"});
    }
    let goblin = goblin_bin();
    if action == "sync" {
        action = "steal".into();
    }
    match action.as_str() {
        "list" => {
            let listing = run_cmd(
                &goblin,
                &["peek", &box_, "--plain"],
                Duration::from_secs(30),
            );
            let text = cmd_text(&listing);
            let limit = fargs
                .get("max_messages")
                .and_then(|x| x.as_i64())
                .unwrap_or(20)
                .clamp(1, 20) as usize;
            let mut rows = Vec::new();
            for line in text.lines().take(limit) {
                if line.starts_with('(') || line.is_empty() {
                    continue;
                }
                let kv = kv_line(line);
                let date = kv.get("date").cloned().unwrap_or_default();
                rows.push(json!({
                    "id": kv.get("id").cloned().unwrap_or_default(),
                    "from": kv.get("from").cloned().unwrap_or_default(),
                    "date": date,
                    "age": age_phrase(&date),
                    "subject": kv.get("subject").cloned().unwrap_or_default(),
                }));
            }
            json!({"action": "list", "box": box_, "count": rows.len(), "rows": rows})
        }
        "hunt" => {
            let q = fargs.get("query").and_then(|x| x.as_str()).unwrap_or("");
            if q.trim().is_empty() {
                return json!({"error": "hunt needs query", "next": "pass query"});
            }
            let listing = run_cmd(&goblin, &["hunt", q, "--plain"], Duration::from_secs(30));
            let text = cmd_text(&listing);
            let mut rows = Vec::new();
            for line in text.lines().take(20) {
                if line.starts_with('(') || line.is_empty() {
                    continue;
                }
                let kv = kv_line(line);
                let date = kv.get("date").cloned().unwrap_or_default();
                rows.push(json!({
                    "id": kv.get("id").cloned().unwrap_or_default(),
                    "from": kv.get("from").cloned().unwrap_or_default(),
                    "date": date,
                    "age": age_phrase(&date),
                    "subject": kv.get("subject").cloned().unwrap_or_default(),
                }));
            }
            json!({"action": "hunt", "query": q, "count": rows.len(), "rows": rows})
        }
        "read" => {
            let id = fargs.get("id").and_then(|x| x.as_str()).unwrap_or("");
            if id.trim().is_empty() {
                return json!({"error": "read needs id", "next": "list first, then read that file id"});
            }
            let got = run_cmd(&goblin, &["read", id], Duration::from_secs(20));
            let mut body = cmd_text(&got);
            if body.len() > 800 {
                body.truncate(800);
                body.push('…');
            }
            json!({"action": "read", "id": id, "body": body})
        }
        "steal" => {
            let got = run_cmd(&goblin, &["steal"], Duration::from_secs(120));
            json!({"action": "sync", "result": cmd_text(&got)})
        }
        "keep" | "trash" => {
            let id = fargs.get("id").and_then(|x| x.as_str()).unwrap_or("");
            if id.trim().is_empty() {
                return json!({"error": format!("{action} needs id")});
            }
            let got = run_cmd(&goblin, &[&action, id], Duration::from_secs(30));
            json!({"action": action, "id": id, "result": cmd_text(&got)})
        }
        "send" => {
            let to = fargs.get("to").and_then(|x| x.as_str()).unwrap_or("");
            let subject = fargs.get("subject").and_then(|x| x.as_str()).unwrap_or("");
            let body = fargs.get("body").and_then(|x| x.as_str()).unwrap_or("");
            if to.trim().is_empty() || subject.trim().is_empty() {
                return json!({"error": "send needs to and subject"});
            }
            let tmp = paths::runtime_dir().join(format!("mail-body-{}", std::process::id()));
            let _ = std::fs::create_dir_all(paths::runtime_dir());
            if std::fs::write(&tmp, body).is_err() {
                return json!({"error": "could not write mail body temp"});
            }
            let got = run_cmd(
                &goblin,
                &[
                    "send",
                    "--to",
                    to,
                    "--subject",
                    subject,
                    "--body-file",
                    &tmp.display().to_string(),
                ],
                Duration::from_secs(60),
            );
            let _ = std::fs::remove_file(&tmp);
            json!({"action": "send", "to": to, "subject": subject, "result": cmd_text(&got)})
        }
        other => json!({"error": format!("unknown mail action {other}")}),
    }
}

fn wacli_bin() -> std::path::PathBuf {
    let p = Path::new("/usr/bin/wacli");
    if p.is_file() {
        p.to_path_buf()
    } else {
        std::path::PathBuf::from("wacli")
    }
}

fn wacli_run(extra: &[&str], read_only: bool) -> Value {
    let bin = wacli_bin();
    let mut args: Vec<&str> = vec!["--json", "--lock-wait", "2s"];
    if read_only {
        args.push("--read-only");
    }
    args.extend_from_slice(extra);
    if extra.iter().any(|a| *a == "--follow") {
        return json!({"error": "wacli --follow is forbidden from Pixie"});
    }
    run_cmd(&bin, &args, Duration::from_secs(45))
}

fn wa_rows_from_json(v: &Value, limit: usize) -> Vec<Value> {
    let stdout = v.get("stdout").and_then(|x| x.as_str()).unwrap_or("");
    let parsed: Value = serde_json::from_str(stdout).unwrap_or(json!({}));
    let msgs = parsed
        .pointer("/data/messages")
        .or_else(|| parsed.get("messages"))
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();
    let mut rows = Vec::new();
    for m in msgs.into_iter().take(limit) {
        let date = m
            .get("Timestamp")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let snippet = m
            .get("Snippet")
            .or_else(|| m.get("Text"))
            .or_else(|| m.get("DisplayText"))
            .and_then(|x| x.as_str())
            .unwrap_or("");
        let snippet: String = snippet.chars().take(160).collect();
        rows.push(json!({
            "id": m.get("MsgID").and_then(|x| x.as_str()).unwrap_or(""),
            "chat": m.get("ChatName").and_then(|x| x.as_str()).unwrap_or(""),
            "from": m.get("SenderName").and_then(|x| x.as_str()).unwrap_or(""),
            "date": date,
            "age": age_phrase(&date),
            "snippet": snippet,
        }));
    }
    rows
}

fn check_whatsapp(fargs: &Value) -> Value {
    let action = fargs
        .get("action")
        .and_then(|x| x.as_str())
        .unwrap_or("list")
        .trim()
        .to_lowercase();
    let limit = fargs
        .get("limit")
        .and_then(|x| x.as_i64())
        .unwrap_or(20)
        .clamp(1, 20) as usize;
    let limit_s = limit.to_string();
    match action.as_str() {
        "list" => {
            let mut extra = vec!["messages", "list", "--limit", limit_s.as_str()];
            let chat = fargs.get("chat").and_then(|x| x.as_str()).unwrap_or("");
            let after = fargs.get("after").and_then(|x| x.as_str()).unwrap_or("");
            let before = fargs.get("before").and_then(|x| x.as_str()).unwrap_or("");
            if !chat.is_empty() {
                extra.extend(["--chat", chat]);
            }
            if !after.is_empty() {
                extra.extend(["--after", after]);
            }
            if !before.is_empty() {
                extra.extend(["--before", before]);
            }
            let raw = wacli_run(&extra, true);
            if extra.contains(&"--follow") {
                return json!({"error": "follow forbidden"});
            }
            let rows = wa_rows_from_json(&raw, limit);
            json!({"action": "list", "count": rows.len(), "rows": rows})
        }
        "search" => {
            let q = fargs.get("query").and_then(|x| x.as_str()).unwrap_or("");
            if q.trim().is_empty() {
                return json!({"error": "search needs query"});
            }
            let extra = ["messages", "search", q, "--limit", limit_s.as_str()];
            let raw = wacli_run(&extra, true);
            let rows = wa_rows_from_json(&raw, limit);
            json!({"action": "search", "query": q, "count": rows.len(), "rows": rows})
        }
        "show" => {
            let id = fargs.get("id").and_then(|x| x.as_str()).unwrap_or("");
            if id.trim().is_empty() {
                return json!({"error": "show needs id"});
            }
            let extra = ["messages", "show", "--id", id];
            let raw = wacli_run(&extra, true);
            let rows = wa_rows_from_json(&raw, 1);
            json!({"action": "show", "id": id, "rows": rows, "raw": cmd_text(&raw).chars().take(800).collect::<String>()})
        }
        "chats" => {
            let extra = ["chats", "list", "--limit", limit_s.as_str()];
            let raw = wacli_run(&extra, true);
            json!({"action": "chats", "result": cmd_text(&raw).chars().take(1200).collect::<String>()})
        }
        "sync" => {
            let extra = ["sync", "--once"];
            let raw = wacli_run(&extra, false);
            json!({"action": "sync", "result": cmd_text(&raw).chars().take(400).collect::<String>()})
        }
        "send" => {
            let to = fargs.get("to").and_then(|x| x.as_str()).unwrap_or("");
            let text = fargs.get("text").and_then(|x| x.as_str()).unwrap_or("");
            if to.trim().is_empty() || text.trim().is_empty() {
                return json!({"error": "send needs to and text"});
            }
            let extra = ["send", "text", "--to", to, "--message", text];
            let raw = wacli_run(&extra, false);
            json!({"action": "send", "to": to, "result": cmd_text(&raw).chars().take(400).collect::<String>()})
        }
        "send_file" => {
            let to = fargs
                .get("to")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            let path = fargs
                .get("path")
                .or_else(|| fargs.get("file"))
                .and_then(|x| x.as_str())
                .unwrap_or("");
            let caption = fargs
                .get("caption")
                .or_else(|| fargs.get("text"))
                .and_then(|x| x.as_str())
                .unwrap_or("");
            if to.trim().is_empty() || path.trim().is_empty() {
                return json!({"error": "send_file needs to and path"});
            }
            let p = match paths::assert_allowed(path) {
                Ok(p) => p,
                Err(e) => return json!({"error": e.to_string()}),
            };
            if !p.is_file() {
                return json!({"error": format!("send_file missing file {}", p.display())});
            }
            let p_s = p.display().to_string();
            let ext = p
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            let mut extra = vec!["send", "file", "--to", to.as_str(), "--file", p_s.as_str()];
            if ext == "wav" {
                extra.extend(["--as", "audio"]);
            }
            if !caption.trim().is_empty() {
                extra.extend(["--caption", caption]);
            }
            let raw = wacli_run(&extra, false);
            json!({
                "action": action,
                "to": to,
                "path": p_s,
                "result": cmd_text(&raw).chars().take(400).collect::<String>()
            })
        }
        "send_voice" => send_voice_note(fargs),
        "mark_read" => {
            let chat = fargs.get("chat").and_then(|x| x.as_str()).unwrap_or("");
            if chat.trim().is_empty() {
                return json!({"error": "mark_read needs chat"});
            }
            let extra = ["chats", "mark-read", "--chat", chat];
            let raw = wacli_run(&extra, false);
            json!({"action": "mark_read", "chat": chat, "result": cmd_text(&raw).chars().take(400).collect::<String>()})
        }
        other => json!({"error": format!("unknown whatsapp action {other}")}),
    }
}

fn send_voice_note(fargs: &Value) -> Value {
    let to = fargs
        .get("to")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    if to.trim().is_empty() {
        return json!({"error": "send_voice needs to"});
    }
    let path = fargs
        .get("path")
        .or_else(|| fargs.get("file"))
        .and_then(|x| x.as_str())
        .unwrap_or("");
    let caption = fargs
        .get("caption")
        .or_else(|| fargs.get("text"))
        .and_then(|x| x.as_str())
        .unwrap_or("");
    if !path.trim().is_empty() {
        if let Err(e) = paths::assert_allowed(path) {
            return json!({"error": e.to_string()});
        }
    }
    if cfg!(test) {
        return json!({
            "action": "send_voice",
            "to": to,
            "source": if !path.trim().is_empty() { "path" } else if !caption.trim().is_empty() { "caption" } else { "last" },
            "result": "test skip send"
        });
    }
    let wav = match resolve_voice_wav(path, caption) {
        Ok(p) => p,
        Err(e) => return json!({"error": e}),
    };
    let note = match wav_to_voice_note(&wav) {
        Ok(p) => p,
        Err(e) => return json!({"error": e}),
    };
    let note_s = note.display().to_string();
    let mut extra = vec![
        "send",
        "file",
        "--to",
        to.as_str(),
        "--file",
        note_s.as_str(),
        "--ptt",
    ];
    if !caption.trim().is_empty() {
        extra.extend(["--caption", caption]);
    }
    let raw = wacli_run(&extra, false);
    json!({
        "action": "send_voice",
        "to": to,
        "path": note_s,
        "result": cmd_text(&raw).chars().take(400).collect::<String>()
    })
}

fn resolve_voice_wav(path: &str, caption: &str) -> Result<std::path::PathBuf, String> {
    if !path.trim().is_empty() {
        let p = paths::assert_allowed(path).map_err(|e| e.to_string())?;
        if !p.is_file() {
            return Err(format!("send_voice missing file {}", p.display()));
        }
        return Ok(p);
    }
    if !caption.trim().is_empty() {
        let dest = crate::speak::cache_wav()
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .join("wa-note.wav");
        crate::speak::synth_to(crate::speak::Tongue::Design, caption, &dest)?;
        if dest.is_file() {
            return Ok(dest);
        }
        return Err("send_voice could not speak the caption".into());
    }
    let last = crate::speak::cache_wav();
    if last.is_file() {
        return Ok(last);
    }
    Err("send_voice needs caption, last speech, or path".into())
}

fn wav_to_voice_note(wav: &std::path::Path) -> Result<std::path::PathBuf, String> {
    let ext = wav
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if ext == "ogg" || ext == "opus" {
        return Ok(wav.to_path_buf());
    }
    let dest = wav.with_extension("ogg");
    let st = std::process::Command::new("ffmpeg")
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
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|e| format!("pixie: ffmpeg: {e}"))?;
    if !st.success() || !dest.is_file() {
        return Err("pixie: could not wrap the clip as an opus voice note".into());
    }
    Ok(dest)
}

fn menagerie_bin() -> std::path::PathBuf {
    let p = paths::home().join("bin/menagerie");
    if p.is_file() {
        p
    } else {
        std::path::PathBuf::from("menagerie")
    }
}

fn hop_pixie(work: impl FnOnce() -> Value) -> Value {
    let men = menagerie_bin();
    crate::forge::set_phase("parking the 8B");
    let _ = Command::new(&men).args(["stop", "pixie"]).output();
    let result = work();
    crate::forge::set_phase("waking Pixie");
    let restore = Command::new(&men).args(["ensure", "pixie"]).output();
    let up = crate::forge::wait_mouth(Duration::from_secs(60));
    crate::forge::clear_phase();
    let restore_err = match restore {
        Ok(o) if o.status.success() => None,
        Ok(o) => {
            let err = String::from_utf8_lossy(&o.stderr);
            Some(format!(
                "the picture may be done but Pixie did not wake: {}",
                err.trim()
            ))
        }
        Err(e) => Some(format!("the picture may be done but Pixie did not wake: {e}")),
    };
    if restore_err.is_none() && !up {
        json!({
            "error": "the picture may be done but Pixie did not wake: mouth still dark",
            "result": result,
        })
    } else if let Some(err) = restore_err {
        json!({
            "error": err,
            "result": result,
        })
    } else {
        result
    }
}

fn hop_eye(work: impl FnOnce() -> Value) -> Value {
    let men = menagerie_bin();
    crate::forge::set_phase("parking the 8B");
    let _ = Command::new(&men).args(["stop", "pixie"]).output();
    crate::forge::set_phase("opening the eye");
    let eye = Command::new(&men)
        .args(["ensure", "pixie", "--slot", "vision"])
        .output();
    let eye_err = match &eye {
        Ok(o) if o.status.success() => None,
        Ok(o) => {
            let err = String::from_utf8_lossy(&o.stderr);
            Some(format!("the eye did not open: {}", err.trim()))
        }
        Err(e) => Some(format!("the eye did not open: {e}")),
    };
    let up = eye_err.is_none() && crate::forge::wait_mouth(Duration::from_secs(60));
    let result = if let Some(err) = eye_err {
        json!({"error": err})
    } else if !up {
        json!({"error": "the eye did not open: mouth still dark"})
    } else {
        crate::forge::set_phase("looking");
        work()
    };
    crate::forge::set_phase("waking Pixie");
    let restore = Command::new(&men)
        .args(["ensure", "pixie", "--slot", "chat"])
        .output();
    let woke = crate::forge::wait_mouth(Duration::from_secs(60));
    crate::forge::clear_phase();
    let restore_err = match restore {
        Ok(o) if o.status.success() => None,
        Ok(o) => {
            let err = String::from_utf8_lossy(&o.stderr);
            Some(format!(
                "the eye spoke but Pixie did not wake: {}",
                err.trim()
            ))
        }
        Err(e) => Some(format!("the eye spoke but Pixie did not wake: {e}")),
    };
    if restore_err.is_none() && !woke {
        json!({
            "error": "the eye spoke but Pixie did not wake: mouth still dark",
            "result": result,
        })
    } else if let Some(err) = restore_err {
        json!({
            "error": err,
            "result": result,
        })
    } else {
        result
    }
}

fn look(path: &str, query: &str, cancel: Option<&AtomicBool>) -> Value {
    let path = path.trim();
    if path.is_empty() {
        return json!({"error": "need a file to look at", "next": "C-u clips a file, or pass path"});
    }
    let low = path.to_ascii_lowercase();
    if matches!(low.as_str(), "screen" | "display" | "desktop") {
        return json!({
            "error": "pixie cannot see the screen yet",
            "next": "clip a file with C-u"
        });
    }
    let p = match paths::assert_allowed(path) {
        Ok(p) => p,
        Err(e) => return json!({"error": e.to_string()}),
    };
    if !p.is_file() {
        return json!({"error": format!("not a file: {}", p.display())});
    }
    match attach::classify(&p) {
        Kind::Other => json!({
            "error": "pixie cannot see this kind yet",
            "path": p.display().to_string(),
            "next": "images and UTF-8 text this cut; PDF later"
        }),
        Kind::Text => {
            let a = match attach::Attach::from_path(&p) {
                Ok(a) => a,
                Err(e) => return json!({"error": e}),
            };
            json!({
                "path": a.path,
                "kind": "text",
                "text": a.excerpt(attach::TEXT_CAP).unwrap_or_default(),
            })
        }
        Kind::Image => {
            let q = query.trim();
            let results = look_images_with(
                &[p.display().to_string()],
                if q.is_empty() {
                    "Describe this image clearly. Name objects, text, colors, and layout. Facts only."
                } else {
                    q
                },
                cancel,
            );
            results
                .into_iter()
                .next()
                .map(|(_, v)| v)
                .unwrap_or_else(|| json!({"error": "the eye was quiet"}))
        }
    }
}

/// One GPU hop for many pictures. Tests never hop.
pub fn look_images(paths: &[String], cancel: Option<&AtomicBool>) -> Vec<(String, Value)> {
    look_images_with(
        paths,
        "Describe this image clearly. Name objects, text, colors, and layout. Facts only.",
        cancel,
    )
}

fn look_images_with(
    paths: &[String],
    prompt: &str,
    cancel: Option<&AtomicBool>,
) -> Vec<(String, Value)> {
    if paths.is_empty() {
        return Vec::new();
    }
    if cfg!(test) {
        return paths
            .iter()
            .map(|p| {
                (
                    p.clone(),
                    json!({"path": p, "text": "test-skip-hop", "skipped": true}),
                )
            })
            .collect();
    }
    if !prefs::mmproj_ready() {
        return paths
            .iter()
            .map(|p| {
                (
                    p.clone(),
                    json!({
                        "path": p,
                        "error": "pixie: the eye is still sleeping",
                        "next": "place Qwen2.5-VL mmproj-f16 next to the VL GGUF"
                    }),
                )
            })
            .collect();
    }
    let packed: Vec<(String, Option<Vec<Value>>)> = paths
        .iter()
        .map(|p| (p.clone(), look_payload(p, prompt)))
        .collect();
    let hop = hop_eye(|| {
        let mut out = Vec::new();
        for (p, payload) in &packed {
            if cancelled(cancel) {
                out.push(json!({"path": p, "error": "cancelled"}));
                continue;
            }
            let Some(msgs) = payload else {
                out.push(json!({"path": p, "error": "could not read the picture"}));
                continue;
            };
            let Some(port) = llm::port() else {
                out.push(json!({"path": p, "error": "no mouth port"}));
                continue;
            };
            match llm::complete_on_if_up_temp(port, msgs, 400, 90, 0.2) {
                Some(text) => out.push(json!({"path": p, "text": text, "kind": "image"})),
                None => out.push(json!({"path": p, "error": "the eye was quiet"})),
            }
        }
        json!({"batch": out})
    });
    let batch = hop
        .get("batch")
        .cloned()
        .or_else(|| hop.get("result").and_then(|r| r.get("batch")).cloned())
        .unwrap_or_else(|| json!([hop]));
    let mut out = Vec::new();
    if let Some(arr) = batch.as_array() {
        for (i, p) in paths.iter().enumerate() {
            let v = arr.get(i).cloned().unwrap_or_else(|| json!({"error": "the eye was quiet"}));
            out.push((p.clone(), v));
        }
    } else {
        for p in paths {
            out.push((p.clone(), hop.clone()));
        }
    }
    out
}

pub(crate) fn look_payload(path: &str, prompt: &str) -> Option<Vec<Value>> {
    let url = image_data_url(Path::new(path))?;
    Some(vec![
        json!({
            "role": "system",
            "content": "You are Pixie's eye. Describe what you see. Facts only. No secrets."
        }),
        json!({
            "role": "user",
            "content": [
                {"type": "text", "text": prompt},
                {"type": "image_url", "image_url": {"url": url}}
            ]
        }),
    ])
}

fn image_data_url(path: &Path) -> Option<String> {
    let img = image::open(path).ok()?;
    let thumb = img.thumbnail(1024, 1024);
    let mut png = Vec::new();
    thumb
        .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
        .ok()?;
    Some(format!(
        "data:image/png;base64,{}",
        kitty::b64_encode(&png)
    ))
}

fn cancelled(cancel: Option<&AtomicBool>) -> bool {
    cancel
        .map(|c| c.load(std::sync::atomic::Ordering::SeqCst))
        .unwrap_or(false)
}

fn generate_image(
    wish: &str,
    cancel: Option<&AtomicBool>,
    confirm: Option<&dyn Fn(&str, &Value) -> bool>,
) -> Value {
    let wish = wish.trim();
    if wish.is_empty() {
        return json!({"error": "need a wish to paint", "next": "say what to draw"});
    }
    crate::forge::set_phase("braiding the wish");
    let mut prompt = crate::forge::enhance_prompt(wish);
    if prompt != wish {
        crate::forge::clear_phase();
        let braid_args = json!({"wish": wish, "prompt": prompt});
        match confirm {
            Some(c) if !c("generate_image", &braid_args) => {
                prompt = wish.to_string();
            }
            None => {}
            _ => {}
        }
        crate::forge::set_phase("braiding the wish");
    }
    if cancelled(cancel) {
        crate::forge::clear_phase();
        return json!({"error": "cancelled"});
    }
    let dir = crate::forge::art_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        crate::forge::clear_phase();
        return json!({"error": format!("could not open the art folder: {e}")});
    }
    let out = crate::forge::stamp_png();
    let hand = match crate::forge::best_hand() {
        Ok(h) => h,
        Err(e) => {
            crate::forge::clear_phase();
            return json!({"error": e, "next": "place SDXL or sd-turbo weights in the models dir"});
        }
    };
    let argv = match crate::forge::sd_argv(&hand, &prompt, &out) {
        Ok(a) => a,
        Err(e) => {
            crate::forge::clear_phase();
            return json!({"error": e});
        }
    };
    let timeout = if hand.kind == "sd-turbo" {
        Duration::from_secs(180)
    } else {
        Duration::from_secs(480)
    };
    let hop = hop_pixie(|| {
        crate::forge::set_phase(&format!(
            "forging {} {}×{} · {} steps",
            hand.kind, hand.width, hand.height, hand.steps
        ));
        match cancel {
            Some(c) => mind::run_argv_cancel(&argv, timeout, c),
            None => mind::run_argv(&argv, timeout),
        }
    });
    let wake_err = hop
        .get("error")
        .and_then(|x| x.as_str())
        .filter(|s| s.contains("did not wake"))
        .map(|s| s.to_string());
    if !out.is_file() {
        let log = cmd_text(&hop);
        return json!({
            "error": if cancelled(cancel) {
                "cancelled"
            } else {
                "the forge wrote no picture"
            },
            "detail": log.chars().take(400).collect::<String>(),
            "wish": wish,
            "prompt": prompt,
        });
    }
    let bytes = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
    crate::forge::write_sidecar(&out, wish, &prompt);
    set_touch(FileTouch {
        path: out.display().to_string(),
        kind: FileKind::Image,
        added: 0,
        deleted: 0,
        lines: Vec::new(),
        binary: true,
        extra_paths: Vec::new(),
        collapsed: false,
    });
    let log = cmd_text(&hop);
    let mut rec = json!({
        "path": out.display().to_string(),
        "wish": wish,
        "prompt": prompt,
        "bytes": bytes,
        "hand": hand.kind,
        "size": format!("{}x{}", hand.width, hand.height),
        "steps": hand.steps,
        "log": log.chars().take(400).collect::<String>(),
    });
    if crate::forge::png_is_blank(&out) {
        rec["error"] = json!("the forge painted a blank canvas");
        rec["next"] = json!("sd-cli argv should match Imp: model, prompt, size, steps, cfg, seed");
    }
    if let Some(e) = wake_err {
        rec["error"] = json!(e);
        rec["next"] = json!("menagerie ensure pixie");
    }
    rec
}

pub fn tool_ok(result: &Value) -> bool {
    result.get("error").is_none()
}

pub fn tool_label(name: &str) -> &str {
    match name {
        "check_mail" => "mail",
        "check_whatsapp" => "whatsapp",
        "generate_image" => "forge",
        "look" => "eye",
        "execute_bash" => "shell",
        "house" => "spell",
        "search_house" => "search",
        "wiki" => "wiki",
        "list_dir" => "looking",
        "grep_files" => "seeking",
        "read_local_file" => "reading",
        "edit_local_file" => "mending",
        "write_local_file" => "writing",
        "remember" => "remembering",
        "manage_systemd_service" => "service",
        "web_research" => "research",
        "fetch_web_page" => "fetch",
        "write_plan" => "plan",
        _ => name,
    }
}

pub fn confirm_line(name: &str, args: &Value) -> String {
    match name {
        "generate_image" => {
            let w = args
                .get("wish")
                .and_then(|x| x.as_str())
                .unwrap_or("this wish");
            if let Some(p) = args.get("prompt").and_then(|x| x.as_str()) {
                if p != w {
                    let clip: String = p.chars().take(80).collect();
                    return format!("paint this?  {clip}  [y/N]");
                }
            }
            let w: String = w.chars().take(60).collect();
            let dir = pretty_home(&crate::prefs::art_dir().display().to_string());
            if crate::prefs::enhance_image() {
                format!("forge this wish (I'll braid it, hop the GPU, save {dir})?  {w}  [y/N]")
            } else {
                format!("forge this wish (hops the GPU, saves {dir})?  {w}  [y/N]")
            }
        }
        "check_whatsapp" => {
            let a = args.get("action").and_then(|x| x.as_str()).unwrap_or("write");
            if a == "send_file" || a == "send_voice" {
                let to = args.get("to").and_then(|x| x.as_str()).unwrap_or("someone");
                let path = args
                    .get("path")
                    .or_else(|| args.get("file"))
                    .and_then(|x| x.as_str())
                    .unwrap_or("");
                if a == "send_voice" && path.trim().is_empty() {
                    let cap = args
                        .get("caption")
                        .or_else(|| args.get("text"))
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .trim();
                    if !cap.is_empty() {
                        let clip: String = cap.chars().take(60).collect();
                        return format!("send a voice note to {to}?  {clip}  [y/N]");
                    }
                    return format!("send a voice note to {to}?  last speech  [y/N]");
                }
                let pretty = pretty_home(if path.trim().is_empty() {
                    "~/.cache/pixie/speak/last.wav"
                } else {
                    path
                });
                return format!("send this file to {to}?  {pretty}  [y/N]");
            }
            format!("let the pocket radio {a}?  [y/N]")
        }
        "check_mail" => {
            let a = args.get("action").and_then(|x| x.as_str()).unwrap_or("write");
            format!("let goblin {a}?  [y/N]")
        }
        "execute_bash" => {
            let c = args.get("command").and_then(|x| x.as_str()).unwrap_or("a command");
            let clip: String = c.chars().take(80).collect();
            format!("run shell?  {clip}  [y/N]")
        }
        _ => format!("approve {}?  [y/N]", tool_label(name)),
    }
}

pub fn tool_story(name: &str, _args: &Value, result: &Value) -> String {
    if let Some(e) = result.get("error") {
        let e = e.as_str().map(|s| s.to_string()).unwrap_or_else(|| e.to_string());
        return format!("that didn't take: {}", e.chars().take(180).collect::<String>());
    }
    match name {
        "look" => result
            .get("text")
            .and_then(|x| x.as_str())
            .unwrap_or("the eye was quiet")
            .chars()
            .take(400)
            .collect(),
        "generate_image" => {
            let path = result.get("path").and_then(|x| x.as_str()).unwrap_or("");
            let size = result.get("size").and_then(|x| x.as_str()).unwrap_or("");
            let hand = result.get("hand").and_then(|x| x.as_str()).unwrap_or("");
            let wish = result.get("wish").and_then(|x| x.as_str()).unwrap_or("");
            let prompt = result.get("prompt").and_then(|x| x.as_str()).unwrap_or("");
            let mut line = format!(
                "a picture, {} · {} · saved {}",
                hand,
                size,
                pretty_home(path)
            );
            if !prompt.is_empty() && prompt != wish {
                line.push('\n');
                line.push_str(prompt);
            }
            line
        }
        "check_mail" | "check_whatsapp" => {
            if let Some(rows) = result.get("rows").and_then(|x| x.as_array()) {
                if rows.is_empty() {
                    return "the nest is quiet.".into();
                }
                let mut lines = Vec::new();
                for r in rows.iter().take(12) {
                    let age = r.get("age").and_then(|x| x.as_str()).unwrap_or("?");
                    let who = r
                        .get("from")
                        .or_else(|| r.get("chat"))
                        .and_then(|x| x.as_str())
                        .unwrap_or("?");
                    let what = r
                        .get("subject")
                        .or_else(|| r.get("snippet"))
                        .and_then(|x| x.as_str())
                        .unwrap_or("");
                    lines.push(format!("{age:<5}  {who}  {what}"));
                }
                return lines.join("\n");
            }
            result
                .get("result")
                .and_then(|x| x.as_str())
                .unwrap_or("done")
                .chars()
                .take(240)
                .collect()
        }
        _ => String::new(),
    }
}

pub fn display_body(name: &str, args: &Value, result: &Value) -> String {
    let story = tool_story(name, args, result);
    if !story.is_empty() {
        story
    } else {
        tool_body(name, args, result, false)
    }
}

fn pretty_home(p: &str) -> String {
    let h = paths::home();
    let hs = h.to_string_lossy();
    if let Some(rest) = p.strip_prefix(hs.as_ref()) {
        format!("~{rest}")
    } else {
        p.to_string()
    }
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
    if alias(name) == "generate_image" {
        let path = result.get("path").and_then(|x| x.as_str())?;
        return Some(FileTouch {
            path: path.into(),
            kind: FileKind::Image,
            added: 0,
            deleted: 0,
            lines: Vec::new(),
            binary: true,
            extra_paths: Vec::new(),
            collapsed: false,
        });
    }
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
                collapsed: false,
            })
        }
        "write_local_file" | "edit_local_file" => {
            let path = result.get("path").and_then(|x| x.as_str())?;
            let created = result
                .get("created")
                .and_then(|x| x.as_bool())
                .unwrap_or(false);
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
                binary: result
                    .get("binary")
                    .and_then(|x| x.as_bool())
                    .unwrap_or(false),
                extra_paths: Vec::new(),
                collapsed: false,
            })
        }
        "search_house" | "grep_files" => {
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
                collapsed: false,
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
                collapsed: false,
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
            return format!(
                "error: {}",
                e.to_string().chars().take(180).collect::<String>()
            );
        }
        if let Some(rows) = result.get("rows").and_then(|x| x.as_array()) {
            if rows.is_empty() {
                return "(none)".into();
            }
            let mut lines = Vec::new();
            for r in rows.iter().take(12) {
                let age = r.get("age").and_then(|x| x.as_str()).unwrap_or("?");
                let date = r.get("date").and_then(|x| x.as_str()).unwrap_or("");
                let when = if age == "?" && !date.is_empty() {
                    date.chars().take(17).collect::<String>()
                } else {
                    age.to_string()
                };
                let who = r
                    .get("from")
                    .or_else(|| r.get("chat"))
                    .and_then(|x| x.as_str())
                    .unwrap_or("?");
                let what = r
                    .get("subject")
                    .or_else(|| r.get("snippet"))
                    .and_then(|x| x.as_str())
                    .unwrap_or("");
                lines.push(format!("{when:<5}  {who}  {what}"));
            }
            return lines.join("\n");
        }
        if let Some(s) = result.get("stdout").and_then(|x| x.as_str()) {
            return s.chars().take(240).collect::<String>();
        }
        if result
            .get("written")
            .and_then(|x| x.as_bool())
            .unwrap_or(false)
            || result
                .get("edited")
                .and_then(|x| x.as_bool())
                .unwrap_or(false)
        {
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
        assert_eq!(alias("apply_patch"), "edit_local_file");
        assert_eq!(alias("str_replace"), "edit_local_file");
        assert_eq!(alias("listdir"), "list_dir");
        assert_eq!(alias("grep"), "grep_files");
        assert_eq!(alias("whatsapp"), "check_whatsapp");
        assert_eq!(alias("wacli"), "check_whatsapp");
        assert_eq!(alias("inbox"), "check_mail");
        assert_eq!(alias("draw"), "generate_image");
    }

    #[test]
    fn mail_default_is_local_list() {
        let n = norm_args("check_mail", &json!({}));
        assert_eq!(n["action"], "list");
        assert_eq!(n["sync"], false);
        assert!(!needs_confirm_args("check_mail", RuneKey::Chat, &n));
        assert!(needs_confirm_args(
            "check_mail",
            RuneKey::Chat,
            &json!({"action": "send"})
        ));
        assert!(needs_confirm_args(
            "check_whatsapp",
            RuneKey::Chat,
            &json!({"action": "sync"})
        ));
        assert!(!needs_confirm_args(
            "check_whatsapp",
            RuneKey::Chat,
            &json!({"action": "list"})
        ));
        assert!(needs_confirm_args(
            "check_whatsapp",
            RuneKey::Chat,
            &json!({"action": "send_file"})
        ));
        assert!(needs_confirm_args(
            "check_whatsapp",
            RuneKey::Chat,
            &json!({"action": "send_voice"})
        ));
        assert!(known_for(RuneKey::Chat).contains(&"check_whatsapp"));
        assert!(!known_for(RuneKey::Deep).contains(&"check_whatsapp"));
        assert!(!known_for(RuneKey::Plan).contains(&"check_whatsapp"));
    }

    #[test]
    fn quiet_mail_rows_are_human() {
        let body = tool_body(
            "check_mail",
            &json!({}),
            &json!({
                "count": 1,
                "rows": [{"age": "2h", "from": "Ada", "subject": "hello", "date": "2026-10-29"}]
            }),
            false,
        );
        assert!(body.contains("Ada"), "{body}");
        assert!(body.contains("hello"), "{body}");
        assert!(!body.contains("Routing:"), "{body}");
    }

    #[test]
    fn wacli_source_forbids_follow() {
        let src = include_str!("tools.rs");
        assert!(src.contains("--follow") && src.contains("forbidden"));
        assert!(src.contains("--read-only"));
        assert!(src.contains("--once"));
        assert!(src.contains("send_file"));
        assert!(src.contains("\"send\", \"file\""));
        assert!(src.contains("--ptt"));
        assert!(src.contains("libopus"));
        assert!(src.contains("wa-note.wav"));
    }

    #[test]
    fn send_file_needs_confirm_and_path_gate() {
        let line = confirm_line(
            "check_whatsapp",
            &json!({"action": "send_file", "to": "Ada", "path": "~/pixie_art/x.png"}),
        );
        assert!(line.contains("send this file"), "{line}");
        assert!(line.contains("Ada"), "{line}");
        let denied = check_whatsapp(&json!({
            "action": "send_file",
            "to": "Ada",
            "path": "~/.ssh/config"
        }));
        let err = denied["error"].as_str().unwrap_or("");
        assert!(
            err.contains("sensitive") || err.contains("refusing"),
            "{denied}"
        );
        let missing_args = check_whatsapp(&json!({"action": "send_file", "to": "Ada"}));
        assert!(
            missing_args["error"]
                .as_str()
                .unwrap_or("")
                .contains("to and path"),
            "{missing_args}"
        );
        let voice = check_whatsapp(&json!({
            "action": "send_voice",
            "to": "Ada",
            "caption": "hello from pixie"
        }));
        assert_eq!(voice["result"], "test skip send", "{voice}");
        assert_eq!(voice["source"], "caption", "{voice}");
        let voice_line = confirm_line(
            "check_whatsapp",
            &json!({"action": "send_voice", "to": "Ada"}),
        );
        assert!(voice_line.contains("voice note"), "{voice_line}");
        assert!(voice_line.contains("last speech"), "{voice_line}");
        let cap_line = confirm_line(
            "check_whatsapp",
            &json!({"action": "send_voice", "to": "Ada", "caption": "hello from pixie"}),
        );
        assert!(cap_line.contains("hello from pixie"), "{cap_line}");
        let n = norm_args(
            "check_whatsapp",
            &json!({
                "action": "send_voice",
                "to": "Ada",
                "caption": "hello from pixie"
            }),
        );
        assert_eq!(n["caption"], "hello from pixie", "{n}");
        assert_eq!(n["text"], "hello from pixie", "{n}");
        let after = check_whatsapp(&n);
        assert_eq!(after["source"], "caption", "{after}");
        let voice_denied = check_whatsapp(&json!({
            "action": "send_voice",
            "to": "Ada",
            "path": "~/.ssh/config"
        }));
        let verr = voice_denied["error"].as_str().unwrap_or("");
        assert!(
            verr.contains("sensitive") || verr.contains("refusing"),
            "{voice_denied}"
        );
        let home = std::env::var("HOME").unwrap_or_else(|_| "/home/evenweaker".into());
        let missing = check_whatsapp(&json!({
            "action": "send_file",
            "to": "Ada",
            "path": format!("{home}/no-such-pixie-voice.wav")
        }));
        assert!(
            missing["error"]
                .as_str()
                .unwrap_or("")
                .contains("missing file"),
            "{missing}"
        );
    }

    #[test]
    fn house_splits_spell_string() {
        let got = norm_args("house", &json!({"spell": "siren play lofi"}));
        assert_eq!(got["spell"], "siren");
        assert_eq!(got["args"], json!(["play", "lofi"]));
    }

    #[test]
    fn chat_bash_honors_and() {
        let yes: &dyn Fn(&str, &Value) -> bool = &|_, _| true;
        let r = run_tool(
            "execute_bash",
            &json!({"command": "echo a && echo b"}),
            RuneKey::Chat,
            "",
            Some(yes),
            None,
        );
        let out = r["stdout"].as_str().unwrap_or("");
        assert!(out.contains('a') && out.contains('b'), "{r}");
        assert_eq!(r["argv"][0], "sh");
        assert_eq!(r["argv"][1], "-c");
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
        assert!(
            msg.contains("read-only") || msg.contains("allowlist"),
            "{msg}"
        );
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
        assert!(
            msg.contains("Unknown tool") || msg.contains("web_research"),
            "{r}"
        );
    }

    #[test]
    fn list_dir_and_grep_are_ours() {
        let dir = paths::cache_dir().join(format!("tool-up-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let p = dir.join("note.txt");
        fs::write(&p, "alpha unique-needle omega\n").unwrap();
        let path = dir.display().to_string();
        let listed = run_tool(
            "list_dir",
            &json!({"path": path}),
            RuneKey::Chat,
            "",
            None,
            None,
        );
        assert!(listed["count"].as_u64().unwrap_or(0) >= 1, "{listed}");
        let grepped = run_tool(
            "grep_files",
            &json!({"path": path, "pattern": "unique-needle"}),
            RuneKey::Chat,
            "",
            None,
            None,
        );
        let blob = grepped.to_string();
        assert!(blob.contains("unique-needle"), "{grepped}");
        let etc = run_tool(
            "list_dir",
            &json!({"path": "/etc"}),
            RuneKey::Chat,
            "",
            None,
            None,
        );
        assert!(etc.get("error").is_some(), "{etc}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn edit_replaces_once_and_plan_refuses() {
        let dir = paths::cache_dir().join(format!("tool-edit-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let p = dir.join("x.txt");
        fs::write(&p, "keep\nold\nalso\n").unwrap();
        let path = p.display().to_string();
        let r = run_tool(
            "apply_patch",
            &json!({"path": path, "old_string": "old", "new_string": "new"}),
            RuneKey::Build,
            "",
            None,
            None,
        );
        assert_eq!(r.get("edited"), Some(&json!(true)), "{r}");
        let text = fs::read_to_string(&p).unwrap();
        assert!(text.contains("new"), "{text}");
        assert!(!text.contains("old"), "{text}");
        let miss = run_tool(
            "edit_local_file",
            &json!({"path": path, "old_string": "nope", "new_string": "x"}),
            RuneKey::Build,
            "",
            None,
            None,
        );
        assert!(miss.get("error").is_some(), "{miss}");
        let plan = run_tool(
            "edit_local_file",
            &json!({"path": path, "old_string": "keep", "new_string": "no"}),
            RuneKey::Plan,
            "",
            None,
            None,
        );
        assert!(
            plan["error"]
                .as_str()
                .unwrap_or("")
                .contains("Unknown tool"),
            "{plan}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn look_skips_hop_in_tests() {
        let screen = run_tool(
            "look",
            &json!({"target": "screen"}),
            RuneKey::Chat,
            "",
            None,
            None,
        );
        assert!(
            screen["error"].as_str().unwrap().contains("screen"),
            "{screen}"
        );
        let missing = paths::cache_dir().join("no-such-pixie-eye.png");
        let r = run_tool(
            "look",
            &json!({"path": missing.display().to_string()}),
            RuneKey::Chat,
            "",
            None,
            None,
        );
        let err = r["error"].as_str().unwrap_or("");
        assert!(err.contains("not a file") || err.contains("sleeping"), "{r}");
    }

    #[test]
    fn look_payload_has_image_url() {
        let dir = paths::cache_dir().join(format!("look-payload-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let p = dir.join("tiny.png");
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            4,
            4,
            image::Rgb([0xe8, 0x79, 0xa0]),
        ))
        .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
        .unwrap();
        fs::write(&p, &bytes).unwrap();
        let msgs = look_payload(&p.display().to_string(), "what is this").expect("payload");
        let blob = format!("{msgs:?}");
        assert!(blob.contains("image_url"), "{blob}");
        assert!(blob.contains("data:image/png;base64,"), "{blob}");
        let _ = fs::remove_dir_all(&dir);
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
        assert!(
            r.get("added").and_then(|x| x.as_u64()).unwrap_or(0) >= 2,
            "{r}"
        );
        assert!(
            r.get("hunks").is_none(),
            "model payload must not dump hunks: {r}"
        );
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
        assert!(
            r2.get("deleted").and_then(|x| x.as_u64()).unwrap_or(0) >= 1,
            "{r2}"
        );
        let t2 = take_touch().expect("edit touch");
        assert!(t2
            .lines
            .iter()
            .any(|l| l.op == Op::Del && l.text.contains("beta")));
        assert!(t2
            .lines
            .iter()
            .any(|l| l.op == Op::Add && l.text.contains("gamma")));
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
    fn forge_cards_are_human() {
        assert_eq!(tool_label("generate_image"), "forge");
        assert_eq!(tool_label("check_whatsapp"), "whatsapp");
        assert_eq!(tool_label("execute_bash"), "shell");
        let line = confirm_line(
            "generate_image",
            &json!({"wish": "a dragon"}),
        );
        assert!(line.contains("braid"), "{line}");
        assert!(line.contains("hop"), "{line}");
        assert!(line.contains("save"), "{line}");
        assert!(line.contains("a dragon"), "{line}");
        let paint = confirm_line(
            "generate_image",
            &json!({
                "wish": "a dragon",
                "prompt": "a huge crimson dragon, moonlight, oil painting"
            }),
        );
        assert!(paint.starts_with("paint this?"), "{paint}");
        assert!(paint.contains("crimson dragon"), "{paint}");
        let story = tool_story(
            "generate_image",
            &json!({"wish": "a dragon"}),
            &json!({
                "path": "/home/evenweaker/pixie_art/pixie-1.png",
                "hand": "sdxl",
                "size": "1024x1024"
            }),
        );
        assert!(story.contains("picture"), "{story}");
        assert!(story.contains("sdxl"), "{story}");
        assert!(!story.contains("check_whatsapp"), "{story}");
        let touch = touch_from_rec(
            "generate_image",
            &json!({"wish": "a dragon"}),
            &json!({"path": "/home/evenweaker/pixie_art/pixie-1.png"}),
        )
        .expect("image card");
        assert_eq!(touch.kind, FileKind::Image);
        assert!(touch.binary);
        assert!(!tool_ok(&json!({"error": "nope"})));
        assert!(tool_ok(&json!({"path": "x.png"})));
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
