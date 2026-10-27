use crate::diff::FileTouch;
use crate::host;
use crate::llm::{self, Abort};
use crate::mind::{self, SessionRec};
use crate::runes::{self, Rune, RuneKey, PERSONA};
use crate::think;
use crate::tools;
use serde_json::{json, Value};
use std::sync::atomic::AtomicBool;
use std::time::Instant;

#[derive(Clone)]
pub enum ChatEntry {
    User(String),
    Assistant {
        text: String,
        clock: String,
        think: String,
        cancelled: bool,
    },
    Tool {
        name: String,
        title: String,
        body: String,
        file: Option<FileTouch>,
        think: String,
    },
    /// Prompt-only recap. TUI skips it; jsonl keeps it.
    Compact {
        text: String,
    },
}

pub struct TurnOut {
    pub text: String,
    pub think: String,
    pub cancelled: bool,
    pub err: Option<String>,
    pub wrote_plan: bool,
}

pub const LOOP_GUARD: usize = 64;
pub const COMPACT_PCT: usize = 70;

fn persona_block() -> String {
    crate::prefs::persona_text().unwrap_or_else(|| PERSONA.to_string())
}

pub fn system_prompt(rune: &Rune, session_id: &str, cwd: &str) -> String {
    let host_card = host::probe();
    let pack = mind::house_pack();
    let mem = mind::load_memory_inject(None, mind::MEMORY_INJECT_CAP);
    let spells = mind::load_spells();
    let tools_block = rune
        .tools
        .iter()
        .enumerate()
        .map(|(i, (n, s))| format!("{}. {n}: {s}", i + 1))
        .collect::<Vec<_>>()
        .join("\n");
    let mut extra = String::new();
    let plan = mind::load_plan(session_id, 3000);
    if !plan.is_empty() {
        extra.push_str("\n\nActive plan (Pixie cache only — not a project markdown file):\n");
        extra.push_str(&plan);
        extra.push_str("\nImplement this in ~/pixie. Do not write ~/.config/plan. Do not tell the human you wrote ~/faeOS/*.md.\n");
    }
    let srcs = mind::listed_sources(session_id);
    if !srcs.is_empty() {
        extra.push_str("\nSources gathered this session:\n");
        for s in srcs.iter().take(16) {
            extra.push_str(&format!(
                "- {} {}\n",
                s.get("title").and_then(|x| x.as_str()).unwrap_or(""),
                s.get("url").and_then(|x| x.as_str()).unwrap_or("")
            ));
        }
    }
    let persona = persona_block();
    format!(
        "{persona}\n\nHost card:\n{host_card}\n\n{pack}\n\n{}\n\nCurrent working directory: {cwd}\nUse real, existing paths.\n\nHouse memory:\n{mem}\n{extra}\n\n{}\n\nAvailable tools in this mode:\n{tools_block}",
        mind::spells_line(&spells),
        rune.prompt
    )
}

pub fn wrap_tool_result(name: &str, body: &str) -> String {
    let cap: String = body.chars().take(1200).collect();
    format!(
        "<tool_result name=\"{name}\">\n{cap}\n</tool_result>\nThis is a TOOL RESULT, not the human. Continue the same task.\nNeed another tool → reply with EXACTLY one JSON object {{\"name\": \"...\", \"arguments\": {{...}}}} and nothing else.\nDone → answer the human. Do not ask which folder unless they asked."
    )
}

pub const HISTORY_CHAR_BUDGET: usize = 24_000;

fn entry_prompt_chars(e: &ChatEntry) -> usize {
    match e {
        ChatEntry::User(t) => t.len(),
        ChatEntry::Assistant { text, .. } => text.len(),
        ChatEntry::Tool { name, body, .. } => wrap_tool_result(name, body).len(),
        ChatEntry::Compact { text } => text.len(),
    }
}

fn last_compact_idx(chat: &[ChatEntry]) -> Option<usize> {
    chat.iter()
        .rposition(|e| matches!(e, ChatEntry::Compact { .. }))
}

pub fn prompt_weight(chat: &[ChatEntry]) -> usize {
    let start = last_compact_idx(chat)
        .map(|i| i.saturating_add(1))
        .unwrap_or(0);
    history_slice(&chat[start..], HISTORY_CHAR_BUDGET)
        .iter()
        .map(entry_prompt_chars)
        .sum()
}

pub fn should_compact(chat: &[ChatEntry]) -> bool {
    prompt_weight(chat) * 100 >= HISTORY_CHAR_BUDGET * COMPACT_PCT
}

pub fn thread_pct(chat: &[ChatEntry]) -> u16 {
    let w = prompt_weight(chat);
    ((w.saturating_mul(100)) / HISTORY_CHAR_BUDGET.max(1)).min(100) as u16
}

/// Recap seed for the 8B compact pass (everything after the last compact, clipped).
pub fn compact_seed_text(chat: &[ChatEntry]) -> String {
    let start = last_compact_idx(chat).map(|i| i + 1).unwrap_or(0);
    let window = history_slice(&chat[start..], HISTORY_CHAR_BUDGET);
    let mut s = String::new();
    for e in window {
        match e {
            ChatEntry::Compact { .. } => {}
            ChatEntry::User(t) => {
                if t.trim().is_empty() {
                    continue;
                }
                if !s.is_empty() {
                    s.push('\n');
                }
                s.push_str("you: ");
                s.extend(t.chars().take(400));
            }
            ChatEntry::Assistant { text, .. } => {
                if text.trim().is_empty() {
                    continue;
                }
                if !s.is_empty() {
                    s.push('\n');
                }
                s.push_str("pixie: ");
                s.extend(text.chars().take(400));
            }
            ChatEntry::Tool { name, body, .. } => {
                if !s.is_empty() {
                    s.push('\n');
                }
                s.push_str("tool:");
                s.push_str(name);
                s.push_str(": ");
                s.extend(body.chars().take(180));
            }
        }
        if s.len() > 12_000 {
            break;
        }
    }
    s
}

/// Ask the 8B for a mid-thread recap. None if the mouth is down or the recap is empty.
pub fn request_compact(chat: &[ChatEntry]) -> Option<String> {
    let seed = compact_seed_text(chat);
    if seed.trim().is_empty() {
        return None;
    }
    let msgs = vec![
        json!({
            "role": "system",
            "content": "Compress this local assistant thread into a recap the model will read instead of the earlier turns. Include the human's goal, decisions, files touched, and unfinished work. House facts only. No secrets, passwords, keys, client names, NIFs, mail URLs, or Wi-Fi. Markdown bullets or short prose. At most 40 lines."
        }),
        json!({"role": "user", "content": seed}),
    ];
    let raw = llm::complete_on_if_up(llm::port()?, &msgs, 400, 30)?;
    let text = mind::scrub_compact(&raw);
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

pub fn commit_compact(chat: &mut Vec<ChatEntry>, session_id: &str, text: String) {
    chat.push(ChatEntry::Compact { text: text.clone() });
    mind::append_session(
        session_id,
        &SessionRec {
            ts: mind::now_ts(),
            role: "compact".into(),
            rune: String::new(),
            text,
            tool: String::new(),
            args: None,
            result: None,
            raw: String::new(),
            think: String::new(),
            cancelled: false,
        },
        None,
    );
}

/// Newest entries that fit `budget` chars. Oldest tools in the window drop first.
pub fn history_slice(chat: &[ChatEntry], budget: usize) -> &[ChatEntry] {
    if chat.is_empty() {
        return chat;
    }
    let mut used = 0usize;
    let mut start = chat.len();
    for i in (0..chat.len()).rev() {
        let n = entry_prompt_chars(&chat[i]);
        if start < chat.len() && used.saturating_add(n) > budget {
            break;
        }
        used = used.saturating_add(n);
        start = i;
    }
    while start + 1 < chat.len() && used > budget {
        if matches!(chat[start], ChatEntry::Tool { .. }) {
            used = used.saturating_sub(entry_prompt_chars(&chat[start]));
            start += 1;
        } else {
            break;
        }
    }
    &chat[start..]
}

pub fn chat_messages(chat: &[ChatEntry], rune: &Rune, session_id: &str) -> Vec<Value> {
    let sys = system_prompt(
        rune,
        session_id,
        &std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| ".".into()),
    );
    let mut msgs = vec![json!({"role": "system", "content": sys})];
    let after = last_compact_idx(chat).map(|i| i + 1).unwrap_or(0);
    if after > 0 {
        if let ChatEntry::Compact { text } = &chat[after - 1] {
            msgs.push(json!({
                "role": "user",
                "content": format!("<compact>\n{text}\n</compact>\nEarlier turns were summarized. Continue the same task. This line is not the human.")
            }));
        }
    }
    let history = history_slice(&chat[after..], HISTORY_CHAR_BUDGET);
    let last_is_human = matches!(history.last(), Some(ChatEntry::User(_)));
    let n = history.len();
    for (i, entry) in history.iter().enumerate() {
        match entry {
            ChatEntry::Compact { .. } => {}
            ChatEntry::Tool { name, body, .. } => {
                msgs.push(json!({"role": "user", "content": wrap_tool_result(name, body)}));
            }
            ChatEntry::User(text) => {
                let mut content = text.clone();
                if last_is_human && i + 1 == n {
                    content.push_str(
                        "\n\n(If you need a tool, your reply must be EXACTLY one JSON object {\"name\": \"one of the available tools above\", \"arguments\": {...}} — nothing else. Otherwise answer the human. Do not open with cute filler — a smaller model adds that.)",
                    );
                }
                msgs.push(json!({"role": "user", "content": content}));
            }
            ChatEntry::Assistant { text, .. } => {
                msgs.push(json!({"role": "assistant", "content": text}));
            }
        }
    }
    msgs
}

pub fn run_turn(
    chat: &mut Vec<ChatEntry>,
    rune: &Rune,
    session_id: &str,
    abort: &Abort,
    verbose: bool,
    confirm: Option<&dyn Fn(&str, &Value) -> bool>,
    mut on_event: impl FnMut(Event),
) -> TurnOut {
    let mut msgs = chat_messages(chat, rune, session_id);
    let round_cap = LOOP_GUARD;
    let mut rounds = 0usize;
    let mut last_call: Option<(String, String)> = None;
    let mut repeat = 0u8;
    let mut ctx_retry = false;
    let mut tools_ok = true;
    let mut nudged = false;
    let mut deep_nudged = false;
    let mut spec_nudged = false;
    let mut fetch_count = 0usize;
    let mut empty_tool = false;
    let mut wrote_plan = false;
    let mut listdir_n = 0usize;
    let mut listdir_home = 0usize;
    let cancel_flag: &AtomicBool = &abort.flag;

    loop {
        on_event(Event::Round {
            n: rounds,
            cap: round_cap,
        });
        let tools = if tools_ok {
            Some(runes::openai_tools(rune))
        } else {
            None
        };
        let mut live = String::new();
        let out = llm::stream_chat(&msgs, tools, abort, |c| {
            live.push_str(c);
            on_event(Event::Chunk(c.to_string()));
        });
        if abort.is_cancelled() {
            return TurnOut {
                text: think::strip(&live),
                think: think::split(&live).think,
                cancelled: true,
                err: None,
                wrote_plan,
            };
        }
        if let Some(err) = &out.err {
            let el = err.to_lowercase();
            if tools_ok && el.contains("400") {
                tools_ok = false;
                continue;
            }
            if !ctx_retry
                && msgs.len() > 2
                && (el.contains("400") || el.contains("context") || el.contains("too long"))
            {
                ctx_retry = true;
                mind::drop_oldest(&mut msgs, 0.5);
                continue;
            }
            return TurnOut {
                text: String::new(),
                think: String::new(),
                cancelled: false,
                err: Some(err.clone()),
                wrote_plan,
            };
        }
        let split = think::split(&out.text);
        let first = split.visible.clone();
        let think_txt = split.think.clone();

        let mut tool_calls = Vec::new();
        for c in mind::extract_tool_calls(&first, 8) {
            tool_calls.push(json!({"name": c.name, "arguments": c.arguments}));
        }

        if tool_calls.is_empty()
            && rune.key == RuneKey::Deep
            && fetch_count == 0
            && !deep_nudged
            && !first.trim().is_empty()
        {
            deep_nudged = true;
            msgs.push(json!({"role": "assistant", "content": first}));
            msgs.push(json!({"role": "user", "content": "<harness>That answer has no fetched page. Call fetch_web_page on a URL from the search results, then answer with Sources. This line is not the human.</harness>"}));
            continue;
        }
        if tool_calls.is_empty()
            && empty_tool
            && fetch_count == 0
            && mind::looks_like_invented_specs(&first)
            && !spec_nudged
            && !first.trim().is_empty()
        {
            spec_nudged = true;
            msgs.push(json!({"role": "assistant", "content": first}));
            msgs.push(json!({"role": "user", "content": "<harness>No source for those numbers. Search the house docs or wiki, or say you do not know. Do not invent sizes or torque. This line is not the human.</harness>"}));
            continue;
        }
        if tool_calls.is_empty()
            && !nudged
            && !first.trim().is_empty()
            && mind::looks_like_narrated_tools(&first)
        {
            nudged = true;
            msgs.push(json!({"role": "assistant", "content": first}));
            msgs.push(json!({"role": "user", "content": "<harness>That was narration, not a tool call. If you need a tool, reply with EXACTLY one JSON object {\"name\": \"...\", \"arguments\": {...}} and nothing else. If you are done, answer the human. This line is not the human.</harness>"}));
            continue;
        }

        if !tool_calls.is_empty() && rounds < round_cap {
            let mut results = Vec::new();
            let mut think_once = think_txt.clone();
            for obj in tool_calls.iter().take(4) {
                let fname = obj.get("name").and_then(|x| x.as_str()).unwrap_or("");
                let fargs = obj.get("arguments").cloned().unwrap_or(json!({}));
                let sig = format!("{fname}{}", fargs);
                if last_call.as_ref() == Some(&(fname.to_string(), fargs.to_string())) {
                    repeat += 1;
                } else {
                    repeat = 0;
                }
                last_call = Some((fname.to_string(), fargs.to_string()));
                if repeat >= 3 {
                    break;
                }
                if fname == "list_dir" {
                    listdir_n += 1;
                    let p = fargs.get("path").and_then(|x| x.as_str()).unwrap_or("");
                    if path_is_homeish(p) {
                        listdir_home += 1;
                    }
                    if listdir_n > 6 || listdir_home > 3 {
                        let result = json!({"error": "too many list_dir. If this is about Pixie, use list_dir ~/pixie/src or search_house. Do not list $HOME."});
                        results.push(json!({"tool": fname, "arguments": fargs, "result": result}));
                        continue;
                    }
                }
                let t1 = Instant::now();
                on_event(Event::ToolStart {
                    name: fname.to_string(),
                    args: fargs.clone(),
                });
                let result = tools::run_tool(
                    fname,
                    &fargs,
                    rune.key,
                    session_id,
                    confirm,
                    Some(cancel_flag),
                );
                if fname == "fetch_web_page" && result.get("error").is_none() {
                    fetch_count += 1;
                }
                if mind::tool_was_empty(&result) {
                    empty_tool = true;
                }
                if fname == "write_plan"
                    && result
                        .get("wrote")
                        .and_then(|x| x.as_bool())
                        .unwrap_or(false)
                {
                    wrote_plan = true;
                }
                mind::append_session(
                    session_id,
                    &SessionRec {
                        ts: mind::now_ts(),
                        role: "tool".into(),
                        rune: rune.key.as_str().into(),
                        text: String::new(),
                        tool: fname.into(),
                        args: Some(fargs.clone()),
                        result: Some(result.clone()),
                        raw: first.chars().take(1200).collect(),
                        think: think_txt.clone(),
                        cancelled: false,
                    },
                    None,
                );
                let body = tools::tool_body(fname, &fargs, &result, verbose);
                let title = format!("{fname} · {:.2}s", t1.elapsed().as_secs_f32());
                let file = tools::take_touch();
                let round_think = std::mem::take(&mut think_once);
                chat.push(ChatEntry::Tool {
                    name: fname.into(),
                    title: title.clone(),
                    body: body.clone(),
                    file: file.clone(),
                    think: round_think.clone(),
                });
                on_event(Event::ToolEnd {
                    title,
                    body,
                    name: fname.to_string(),
                    result: result.clone(),
                    file,
                    think: round_think,
                });
                if abort.is_cancelled() {
                    return TurnOut {
                        text: String::new(),
                        think: think_txt,
                        cancelled: true,
                        err: None,
                        wrote_plan,
                    };
                }
                results.push(json!({"tool": fname, "arguments": fargs, "result": result}));
                let _ = sig;
            }
            if !results.is_empty() {
                msgs.push(json!({"role": "assistant", "content": first}));
                let names: Vec<&str> = results
                    .iter()
                    .filter_map(|r| r.get("tool").and_then(|t| t.as_str()))
                    .collect();
                let joined = if names.is_empty() {
                    "tools".into()
                } else {
                    names.join(",")
                };
                msgs.push(json!({
                    "role": "user",
                    "content": wrap_tool_result(&joined, &tools::tool_payload(&results))
                }));
                rounds += 1;
                continue;
            }
        }

        return TurnOut {
            text: first,
            think: think_txt,
            cancelled: false,
            err: None,
            wrote_plan,
        };
    }
}

pub enum Event {
    Chunk(String),
    Round {
        n: usize,
        cap: usize,
    },
    ToolStart {
        name: String,
        #[allow(dead_code)]
        args: Value,
    },
    ToolEnd {
        title: String,
        body: String,
        #[allow(dead_code)]
        name: String,
        #[allow(dead_code)]
        result: Value,
        file: Option<FileTouch>,
        think: String,
    },
}

fn path_is_homeish(p: &str) -> bool {
    let home = crate::paths::home();
    let home_s = home.display().to_string();
    let t = p.trim().trim_end_matches('/');
    let t = t
        .strip_prefix("~/")
        .map(|r| format!("{home_s}/{r}"))
        .unwrap_or_else(|| {
            if t == "~" {
                home_s.clone()
            } else {
                t.to_string()
            }
        });
    t == home_s || t == format!("{home_s}/.config") || t.starts_with(&format!("{home_s}/.config/"))
}

pub fn clock_now() -> String {
    let t = mind::now_ts();
    if let Some((_, rest)) = t.split_once('T') {
        rest.chars().take(8).collect()
    } else {
        t.chars().take(8).collect()
    }
}

pub fn from_session(id: &str) -> (Vec<ChatEntry>, RuneKey) {
    let recs = mind::read_session(id, None);
    let mut chat = Vec::new();
    let mut last = RuneKey::Chat;
    let mut last_tool_think = String::new();
    for rec in recs {
        if let Some(r) = rec.get("rune").and_then(|x| x.as_str()) {
            if let Some(k) = RuneKey::from_str(r) {
                last = k;
            }
        }
        match rec.get("role").and_then(|x| x.as_str()).unwrap_or("") {
            "user" => chat.push(ChatEntry::User(
                rec.get("text")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .into(),
            )),
            "assistant" => chat.push(ChatEntry::Assistant {
                text: rec
                    .get("text")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .into(),
                clock: rec
                    .get("ts")
                    .and_then(|x| x.as_str())
                    .map(|ts| ts.split('T').nth(1).unwrap_or(ts).chars().take(8).collect())
                    .unwrap_or_else(|| "resumed".into()),
                think: rec
                    .get("think")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .into(),
                cancelled: rec
                    .get("cancelled")
                    .and_then(|x| x.as_bool())
                    .unwrap_or(false),
            }),
            "compact" => {
                chat.push(ChatEntry::Compact {
                    text: rec
                        .get("text")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .into(),
                });
            }
            "tool" => {
                let name = rec.get("tool").and_then(|x| x.as_str()).unwrap_or("tool");
                let args = rec.get("args").cloned().unwrap_or(json!({}));
                let result = rec.get("result").cloned().unwrap_or(json!({}));
                let mut think = rec
                    .get("think")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                if !think.is_empty() && think == last_tool_think {
                    think.clear();
                } else if !think.is_empty() {
                    last_tool_think = think.clone();
                }
                chat.push(ChatEntry::Tool {
                    name: name.into(),
                    title: name.to_string(),
                    body: tools::tool_body(name, &args, &result, true),
                    file: tools::touch_from_rec(name, &args, &result),
                    think,
                });
            }
            _ => {}
        }
    }
    (chat, last)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runes::RuneKey;

    #[test]
    fn chat_messages_keeps_every_tool() {
        let rune = runes::by_key(RuneKey::Chat);
        let chat = vec![
            ChatEntry::User("hi".into()),
            ChatEntry::Tool {
                name: "list_dir".into(),
                title: "t1".into(),
                body: "first-tool-body".into(),
                file: None,
                think: String::new(),
            },
            ChatEntry::Tool {
                name: "grep_files".into(),
                title: "t2".into(),
                body: "second-tool-body".into(),
                file: None,
                think: String::new(),
            },
        ];
        let msgs = chat_messages(&chat, &rune, "test");
        let blob = format!("{msgs:?}");
        assert!(blob.contains("first-tool-body"), "{blob}");
        assert!(blob.contains("second-tool-body"), "{blob}");
        assert!(blob.contains("<tool_result"), "{blob}");
        let roles: Vec<&str> = msgs
            .iter()
            .filter_map(|m| m.get("role").and_then(|r| r.as_str()))
            .collect();
        assert!(!roles.iter().any(|r| *r == "tool"), "{roles:?}");
    }

    #[test]
    fn tool_keeps_round_think() {
        let e = ChatEntry::Tool {
            name: "execute_bash".into(),
            title: "execute_bash · 0.1s".into(),
            body: "ok".into(),
            file: None,
            think: "I should list the crate".into(),
        };
        match e {
            ChatEntry::Tool { think, name, .. } => {
                assert_eq!(name, "execute_bash");
                assert_eq!(think, "I should list the crate");
            }
            _ => panic!("tool"),
        }
    }

    #[test]
    fn history_slice_keeps_latest_user() {
        let mut chat = Vec::new();
        for i in 0..12 {
            chat.push(ChatEntry::Tool {
                name: "execute_bash".into(),
                title: "t".into(),
                body: "x".repeat(3000),
                file: None,
                think: String::new(),
            });
            let _ = i;
        }
        chat.push(ChatEntry::User("keep me".into()));
        let w = history_slice(&chat, 8_000);
        assert!(matches!(w.last(), Some(ChatEntry::User(t)) if t == "keep me"));
        assert!(w.len() < chat.len(), "old tools must drop, got {}", w.len());
    }

    #[test]
    fn compact_trigger_on_heavy_window() {
        let mut chat = vec![ChatEntry::User("go".into())];
        for _ in 0..20 {
            chat.push(ChatEntry::Tool {
                name: "execute_bash".into(),
                title: "t".into(),
                body: "y".repeat(2000),
                file: None,
                think: String::new(),
            });
        }
        assert!(should_compact(&chat));
        let seed = compact_seed_text(&chat);
        assert!(seed.contains("tool:execute_bash"), "{seed}");
        chat.push(ChatEntry::Compact {
            text: "earlier work summarized".into(),
        });
        chat.push(ChatEntry::User("and then".into()));
        assert!(!should_compact(&chat));
        let after = compact_seed_text(&chat);
        assert!(after.contains("you: and then"), "{after}");
        assert!(!after.contains("tool:execute_bash"), "{after}");
        assert!(thread_pct(&chat) < 70);
    }

    #[test]
    fn scrub_compact_drops_radio() {
        let s = crate::mind::scrub_compact(
            "- Host is Arch\n- wifi password hunter2\n- Keep working on Pixie\n",
        );
        assert!(s.contains("Host is Arch"));
        assert!(s.contains("Keep working on Pixie"));
        assert!(!s.to_lowercase().contains("wifi"));
        assert!(!s.to_lowercase().contains("password"));
    }

    #[test]
    fn wrap_tool_result_is_not_the_human() {
        let s = wrap_tool_result("list_dir", "entries");
        assert!(s.contains("<tool_result name=\"list_dir\">"));
        assert!(s.contains("not the human"));
    }

    #[test]
    fn system_prompt_includes_persona() {
        let rune = runes::by_key(RuneKey::Chat);
        let p = system_prompt(&rune, "test", ".");
        assert!(p.contains("Pixie"), "{p}");
        if crate::prefs::persona_text().is_none() {
            assert!(p.contains("faeOS familiar"), "{p}");
        }
    }

    #[test]
    fn homeish_is_home_not_pixie() {
        assert!(path_is_homeish("~"));
        assert!(path_is_homeish("/home/evenweaker"));
        assert!(path_is_homeish("/home/evenweaker/.config"));
        assert!(!path_is_homeish("/home/evenweaker/pixie"));
        assert!(!path_is_homeish("/home/evenweaker/pixie/src"));
    }
}
