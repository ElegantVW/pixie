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
    Assistant { text: String, clock: String, think: String, cancelled: bool },
    Tool {
        title: String,
        body: String,
        file: Option<FileTouch>,
    },
}

pub struct TurnOut {
    pub text: String,
    pub think: String,
    pub cancelled: bool,
    pub err: Option<String>,
    pub wrote_plan: bool,
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
        extra.push_str("\n\nActive plan:\n");
        extra.push_str(&plan);
        extra.push('\n');
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
    format!(
        "{PERSONA}\n\nHost card:\n{host_card}\n\n{pack}\n\n{}\n\nCurrent working directory: {cwd}\nUse real, existing paths.\n\nHouse memory:\n{mem}\n{extra}\n\n{}\n\nAvailable tools in this mode:\n{tools_block}",
        mind::spells_line(&spells),
        rune.prompt
    )
}

pub fn chat_messages(chat: &[ChatEntry], rune: &Rune, session_id: &str) -> Vec<Value> {
    let sys = system_prompt(rune, session_id, &std::env::current_dir().map(|p| p.display().to_string()).unwrap_or_else(|_| ".".into()));
    let mut msgs = vec![json!({"role": "system", "content": sys})];
    let start = chat.len().saturating_sub(14);
    let history = &chat[start..];
    let n = history.len();
    for (i, entry) in history.iter().enumerate() {
        match entry {
            ChatEntry::Tool { body, .. } => {
                let cap = body.chars().take(1200).collect::<String>();
                msgs.push(json!({"role": "tool", "content": cap}));
            }
            ChatEntry::User(text) => {
                let mut content = text.clone();
                if i == n - 1 {
                    content.push_str(
                        "\n\n(If you need a tool, your reply must be EXACTLY one JSON object {\"name\": \"one of the available tools above\", \"arguments\": {...}} — nothing else. Otherwise reply conversationally with your pixie charm.)",
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
    let round_cap = rune.rounds;
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
            if !ctx_retry && msgs.len() > 2 && (el.contains("400") || el.contains("context") || el.contains("too long")) {
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
            msgs.push(json!({"role": "user", "content": "That answer has no fetched page. Call fetch_web_page on a URL from the search results, then answer with Sources."}));
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
            msgs.push(json!({"role": "user", "content": "No source for those numbers. Search the house docs or wiki, or say you do not know. Do not invent sizes or torque."}));
            continue;
        }
        if tool_calls.is_empty()
            && !nudged
            && !first.trim().is_empty()
            && mind::looks_like_narrated_tools(&first)
        {
            nudged = true;
            msgs.push(json!({"role": "assistant", "content": first}));
            msgs.push(json!({"role": "user", "content": "That was narration, not a tool call. If you need a tool, reply with EXACTLY one JSON object {\"name\": \"...\", \"arguments\": {...}} and nothing else. If you are done, answer conversationally."}));
            continue;
        }

        if !tool_calls.is_empty() && rounds < round_cap {
            let mut results = Vec::new();
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
                let t1 = Instant::now();
                on_event(Event::ToolStart {
                    name: fname.to_string(),
                    args: fargs.clone(),
                });
                let result = tools::run_tool(fname, &fargs, rune.key, session_id, confirm, Some(cancel_flag));
                if fname == "fetch_web_page" && result.get("error").is_none() {
                    fetch_count += 1;
                }
                if mind::tool_was_empty(&result) {
                    empty_tool = true;
                }
                if fname == "write_plan" && result.get("wrote").and_then(|x| x.as_bool()).unwrap_or(false) {
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
                let title = format!("Calling tools ─ ✦ thought for {:.2}s", t1.elapsed().as_secs_f32());
                let file = tools::take_touch();
                chat.push(ChatEntry::Tool {
                    title: title.clone(),
                    body: body.clone(),
                    file: file.clone(),
                });
                on_event(Event::ToolEnd {
                    title,
                    body,
                    name: fname.to_string(),
                    result: result.clone(),
                    file,
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
                msgs.push(json!({"role": "tool", "content": tools::tool_payload(&results)}));
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
    Round { n: usize, cap: usize },
    ToolStart { name: String, #[allow(dead_code)] args: Value },
    ToolEnd {
        title: String,
        body: String,
        #[allow(dead_code)]
        name: String,
        #[allow(dead_code)]
        result: Value,
        file: Option<FileTouch>,
    },
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
    for rec in recs {
        if let Some(r) = rec.get("rune").and_then(|x| x.as_str()) {
            if let Some(k) = RuneKey::from_str(r) {
                last = k;
            }
        }
        match rec.get("role").and_then(|x| x.as_str()).unwrap_or("") {
            "user" => chat.push(ChatEntry::User(
                rec.get("text").and_then(|x| x.as_str()).unwrap_or("").into(),
            )),
            "assistant" => chat.push(ChatEntry::Assistant {
                text: rec.get("text").and_then(|x| x.as_str()).unwrap_or("").into(),
                clock: rec
                    .get("ts")
                    .and_then(|x| x.as_str())
                    .map(|ts| {
                        ts.split('T').nth(1).unwrap_or(ts).chars().take(8).collect()
                    })
                    .unwrap_or_else(|| "resumed".into()),
                think: rec.get("think").and_then(|x| x.as_str()).unwrap_or("").into(),
                cancelled: rec.get("cancelled").and_then(|x| x.as_bool()).unwrap_or(false),
            }),
            "tool" => {
                let name = rec.get("tool").and_then(|x| x.as_str()).unwrap_or("tool");
                let args = rec.get("args").cloned().unwrap_or(json!({}));
                let result = rec.get("result").cloned().unwrap_or(json!({}));
                chat.push(ChatEntry::Tool {
                    title: format!("Calling tools · {name}"),
                    body: tools::tool_body(name, &args, &result, true),
                    file: tools::touch_from_rec(name, &args, &result),
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
                title: "t1".into(),
                body: "first-tool-body".into(),
                file: None,
            },
            ChatEntry::Tool {
                title: "t2".into(),
                body: "second-tool-body".into(),
                file: None,
            },
        ];
        let msgs = chat_messages(&chat, &rune, "test");
        let blob = format!("{msgs:?}");
        assert!(blob.contains("first-tool-body"), "{blob}");
        assert!(blob.contains("second-tool-body"), "{blob}");
    }
}
