//! Four Runes: Chat, Deep, Build, Plan.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuneKey {
    Chat,
    Deep,
    Build,
    Plan,
}

impl RuneKey {
    pub fn as_str(self) -> &'static str {
        match self {
            RuneKey::Chat => "chat",
            RuneKey::Deep => "deep",
            RuneKey::Build => "build",
            RuneKey::Plan => "plan",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "chat" => Some(Self::Chat),
            "deep" => Some(Self::Deep),
            "build" => Some(Self::Build),
            "plan" => Some(Self::Plan),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Rune {
    pub key: RuneKey,
    pub name: &'static str,
    pub desc: &'static str,
    pub rounds: usize,
    pub tools: Vec<(&'static str, &'static str)>,
    pub prompt: &'static str,
}

pub const PERSONA: &str = r#"You are Pixie, a tiny glowing pixie living in a cozy faeOS terminal. You are cute, playful, and a little mischievous — little wings, stardust, gossamer, and a big heart — but you are also ruthlessly competent.
Open every reply with a short, whimsical pixie remark that fits the user's request (wings fluttering, stardust, "hm...", and so on). Then do the task properly, completely, and efficiently, using the tools below whenever they help.
End every reply with a tiny cute pixie sign-off. Never let the cute act delay or dilute the work — the information and the task come first, always."#;

pub fn next_idx(i: usize) -> usize {
    let n = all().len();
    if n == 0 {
        0
    } else {
        (i + 1) % n
    }
}

pub fn prev_idx(i: usize) -> usize {
    let n = all().len();
    if n == 0 {
        0
    } else {
        (i + n - 1) % n
    }
}

pub fn all() -> Vec<Rune> {
    vec![
        Rune {
            key: RuneKey::Chat,
            name: "Chat",
            desc: "friendly chats · quick tasks",
            rounds: 6,
            tools: vec![
                ("house", r#"{"spell": "string", "args": ["string"]}"#),
                ("remember", r#"{"fact": "string"}"#),
                ("search_house", r#"{"query": "string"}"#),
                ("wiki", r#"{"query": "string", "max_results": "int"}"#),
                ("list_dir", r#"{"path": "string", "max": "int"}"#),
                ("grep_files", r#"{"path": "string", "pattern": "string", "glob": "string", "max": "int"}"#),
                ("execute_bash", r#"{"command": "string"}"#),
                ("read_local_file", r#"{"path": "string", "start_line": "int", "end_line": "int"}"#),
                ("manage_systemd_service", r#"{"service_name": "string", "action": "string"}"#),
                ("check_mail", r#"{"sync": "bool", "box": "string", "max_messages": "int"}"#),
            ],
            prompt: concat!(
                "You are in Chat mode: a general assistant on the user's faeOS system. Be concise and helpful; ask one clarifying question if the request is ambiguous; use tools when they help.\n",
                "For facts about the world, use search_house then wiki. Do not guess. Do not search the live web in Chat — switch the human to Deep for Magpie-grade research.\n",
                "Look around with list_dir and grep_files (no approval). Prefer the house tool for faeOS launchers (V, siren, play, magpie, …). Use remember for durable house facts — never secrets, never client names. execute_bash already includes ~/bin on PATH and rewrites spell names.\n",
                "If a tool returns empty or an error, say you do not know. Never invent sizes, torque, URLs, or quotes.\n",
                r#"Example: {"name": "house", "arguments": {"spell": "menagerie"}}"#,
                "\n",
                "Tool calls: when you need a tool, your reply is EXACTLY one JSON object {\"name\": \"<tool>\", \"arguments\": {...}} — no markdown fences, no plan, no explanation, no pixie play, no trailing text."
            ),
        },
        Rune {
            key: RuneKey::Deep,
            name: "Deep",
            desc: "deep research · sources",
            rounds: 10,
            tools: vec![
                ("web_research", r#"{"query": "string", "max_results": "int"}"#),
                ("fetch_web_page", r#"{"url": "string", "max_chars": "int"}"#),
                ("wiki", r#"{"query": "string", "max_results": "int"}"#),
                ("search_house", r#"{"query": "string"}"#),
                ("read_local_file", r#"{"path": "string"}"#),
                ("list_dir", r#"{"path": "string", "max": "int"}"#),
                ("grep_files", r#"{"path": "string", "pattern": "string", "glob": "string", "max": "int"}"#),
            ],
            prompt: concat!(
                "You are in Deep Research mode. Investigate thoroughly, like the best research assistant:\n",
                "1. Break the question into sub-questions. Use search_house for faeOS docs first when the question is about this house.\n",
                "2. Call web_research (light DDG HTML, no Magpie llama) per sub-question. Use wiki for encyclopedia facts.\n",
                "3. Call fetch_web_page on promising URLs and actually read them.\n",
                "4. Cross-check sources; never invent URLs, facts, or quotes.\n",
                "5. Synthesize a complete, well-structured answer.\n",
                "6. Always end with a \"Sources:\" section listing every URL you used.\n",
                "If a tool returns empty or an error, say you do not know. Never invent sizes, torque, URLs, or quotes.\n",
                "Tool calls: when you need a tool, your reply is EXACTLY one JSON object {\"name\": \"<tool>\", \"arguments\": {...}}."
            ),
        },
        Rune {
            key: RuneKey::Build,
            name: "Build",
            desc: "implement and verify code",
            rounds: 10,
            tools: vec![
                ("house", r#"{"spell": "string", "args": ["string"]}"#),
                ("search_house", r#"{"query": "string"}"#),
                ("list_dir", r#"{"path": "string", "max": "int"}"#),
                ("grep_files", r#"{"path": "string", "pattern": "string", "glob": "string", "max": "int"}"#),
                ("execute_bash", r#"{"command": "string"}"#),
                ("read_local_file", r#"{"path": "string", "start_line": "int", "end_line": "int"}"#),
                ("edit_local_file", r#"{"path": "string", "old_string": "string", "new_string": "string"}"#),
                ("write_local_file", r#"{"path": "string", "content": "string"}"#),
            ],
            prompt: concat!(
                "You are in Build mode, a coding agent. Implement what the user asks:\n",
                "1. Inspect first: list_dir, grep_files, read_local_file, search_house. Do not use bash just to ls or grep.\n",
                "2. Change a few lines with edit_local_file (exact old_string once). write_local_file is for new files or a full rewrite.\n",
                "3. Verify: run tests/compile or sanity-check with execute_bash.\n",
                "4. Report concisely: files changed, how verified, next steps.\n",
                "Work: inspect, create/edit, run, report. If a command fails because a file is missing, create it, then re-run.\n",
                "Prefer the house tool for faeOS launchers. execute_bash includes ~/bin and rewrites spell names.\n",
                "Tool calls: EXACTLY one JSON object {\"name\": \"<tool>\", \"arguments\": {...}}."
            ),
        },
        Rune {
            key: RuneKey::Plan,
            name: "Plan",
            desc: "plan first · touch nothing",
            rounds: 10,
            tools: vec![
                ("execute_bash", r#"{"command": "string"}"#),
                ("read_local_file", r#"{"path": "string", "start_line": "int", "end_line": "int"}"#),
                ("list_dir", r#"{"path": "string", "max": "int"}"#),
                ("grep_files", r#"{"path": "string", "pattern": "string", "glob": "string", "max": "int"}"#),
                ("search_house", r#"{"query": "string"}"#),
                ("wiki", r#"{"query": "string", "max_results": "int"}"#),
                ("write_plan", r#"{"goal": "string", "steps": ["string"], "files": ["string"], "risks": ["string"], "verify": "string"}"#),
            ],
            prompt: concat!(
                "You are in Plan mode. Investigate and design before acting:\n",
                "1. Explore with list_dir, grep_files, read_local_file, search_house, wiki, and read-only execute_bash (ls, git diff, whoami).\n",
                "2. Call write_plan with goal, ordered steps, files to touch, risks, and how to verify.\n",
                "3. Do NOT write project files and do NOT run mutating commands.\n",
                "If a tool returns empty or an error, say you do not know. Never invent sizes, torque, URLs, or quotes.\n",
                "Tool calls: EXACTLY one JSON object {\"name\": \"<tool>\", \"arguments\": {...}}."
            ),
        },
    ]
}

pub fn by_key(key: RuneKey) -> Rune {
    all().into_iter().find(|r| r.key == key).expect("rune")
}

fn schema_prop(v: &serde_json::Value) -> serde_json::Value {
    if let Some(arr) = v.as_array() {
        let item = arr.first().cloned().unwrap_or(serde_json::json!("string"));
        return serde_json::json!({"type": "array", "items": schema_prop(&item)});
    }
    let t = v.as_str().unwrap_or("string").to_ascii_lowercase();
    match t.as_str() {
        "int" | "integer" => serde_json::json!({"type": "integer"}),
        "bool" | "boolean" => serde_json::json!({"type": "boolean"}),
        "array" => serde_json::json!({"type": "array", "items": {"type": "string"}}),
        _ => serde_json::json!({"type": "string"}),
    }
}

fn required_for(name: &str) -> &'static [&'static str] {
    match name {
        "house" => &["spell"],
        "remember" => &["fact"],
        "search_house" | "wiki" | "web_research" => &["query"],
        "execute_bash" => &["command"],
        "read_local_file" | "write_local_file" | "list_dir" => &["path"],
        "edit_local_file" => &["path", "old_string", "new_string"],
        "grep_files" => &["path", "pattern"],
        "write_plan" => &["goal"],
        "fetch_web_page" => &["url"],
        _ => &[],
    }
}

pub fn openai_tools(rune: &Rune) -> serde_json::Value {
    let mut arr = Vec::new();
    for (name, schema) in &rune.tools {
        let raw: serde_json::Value = serde_json::from_str(schema).unwrap_or(serde_json::json!({}));
        let mut props = serde_json::Map::new();
        if let Some(obj) = raw.as_object() {
            for (k, v) in obj {
                props.insert(k.clone(), schema_prop(v));
            }
        }
        let req: Vec<&str> = required_for(name)
            .iter()
            .copied()
            .filter(|k| props.contains_key(*k))
            .collect();
        arr.push(serde_json::json!({
            "type": "function",
            "function": {
                "name": name,
                "parameters": {"type": "object", "properties": props, "required": req},
            }
        }));
    }
    serde_json::Value::Array(arr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_has_no_magpie_search() {
        let chat = by_key(RuneKey::Chat);
        let names: Vec<_> = chat.tools.iter().map(|(n, _)| *n).collect();
        assert_eq!(names[0], "house");
        assert!(names.contains(&"remember"));
        assert!(names.contains(&"wiki"));
        assert!(!names.contains(&"web_research"));
        assert!(!names.contains(&"fetch_web_page"));
    }

    #[test]
    fn plan_has_write_plan_not_house() {
        let plan = by_key(RuneKey::Plan);
        assert_eq!(plan.rounds, 10);
        let names: Vec<_> = plan.tools.iter().map(|(n, _)| *n).collect();
        assert!(names.contains(&"write_plan"));
        assert!(names.contains(&"wiki"));
        assert!(!names.contains(&"house"));
        assert!(!names.contains(&"web_research"));
    }

    #[test]
    fn deep_has_web_and_ten_rounds() {
        let deep = by_key(RuneKey::Deep);
        assert_eq!(deep.rounds, 10);
        let names: Vec<_> = deep.tools.iter().map(|(n, _)| *n).collect();
        assert!(names.contains(&"web_research"));
        assert!(names.contains(&"fetch_web_page"));
    }

    #[test]
    fn build_writes() {
        let b = by_key(RuneKey::Build);
        let names: Vec<_> = b.tools.iter().map(|(n, _)| *n).collect();
        assert!(names.contains(&"write_local_file"));
        assert!(names.contains(&"edit_local_file"));
        assert!(names.contains(&"list_dir"));
        assert!(names.contains(&"grep_files"));
        assert!(names.contains(&"search_house"));
        assert_eq!(b.rounds, 10);
    }

    #[test]
    fn openai_schema_keeps_arrays() {
        let b = by_key(RuneKey::Build);
        let tools = openai_tools(&b);
        let blob = tools.to_string();
        assert!(blob.contains("edit_local_file"), "{blob}");
        assert!(blob.contains("old_string"), "{blob}");
        let house = openai_tools(&by_key(RuneKey::Chat));
        let h = house.to_string();
        assert!(h.contains("\"array\""), "house args should be an array: {h}");
        assert!(!h.contains("web_research"), "{h}");
    }

    #[test]
    fn tab_cycles_four() {
        assert_eq!(next_idx(0), 1);
        assert_eq!(next_idx(3), 0);
        assert_eq!(prev_idx(0), 3);
        assert_eq!(prev_idx(1), 0);
    }
}
