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

    pub fn next(self) -> Self {
        match self {
            Self::Chat => Self::Deep,
            Self::Deep => Self::Build,
            Self::Build => Self::Plan,
            Self::Plan => Self::Chat,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Rune {
    pub key: RuneKey,
    pub name: &'static str,
    pub desc: &'static str,
    #[allow(dead_code)]
    pub rounds: usize,
    pub tools: Vec<(&'static str, &'static str)>,
    pub prompt: &'static str,
}

pub const PERSONA: &str = r#"You are Pixie, the faeOS familiar. Competent first, then a little fairy. All-ages. Cute after you have the facts.
When you need a tool, reply with EXACTLY one JSON object {"name":"...","arguments":{...}} and nothing else. No greeting, no markdown fence, no plan-in-prose.
When the human's request is done, answer in your own voice with the facts intact. Do not invent USB pads from /dev/input/event*. Do not list $HOME unless they asked. If the topic is Pixie/Plan/Build, inspect ~/pixie (search_house, list_dir ~/pixie/src)."#;

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

pub fn idx_of(key: RuneKey) -> usize {
    all().iter().position(|r| r.key == key).unwrap_or(0)
}

pub fn all() -> Vec<Rune> {
    vec![
        Rune {
            key: RuneKey::Chat,
            name: "Chat",
            desc: "friendly chats · quick tasks",
            rounds: 12,
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
                ("check_mail", r#"{"action": "string", "box": "string", "query": "string", "id": "string", "to": "string", "subject": "string", "body": "string", "sync": "bool", "max_messages": "int"}"#),
                ("check_whatsapp", r#"{"action": "string", "query": "string", "chat": "string", "id": "string", "to": "string", "text": "string", "after": "string", "before": "string", "limit": "int"}"#),
                ("generate_image", r#"{"wish": "string"}"#),
                ("look", r#"{"path": "string", "query": "string"}"#),
            ],
            prompt: concat!(
                "You are in Chat mode: house familiar. Concise. One clarifying question only if the request is ambiguous.\n",
                "list_dir / grep_files / search_house for files. execute_bash for date, tests, systemctl — it is a real zsh-capable shell (&& works).\n",
                "USB/gamepads: read /proc/bus/input/devices or list_dir /dev/input/by-id. event0..eventN are not pads.\n",
                "Prefer the house tool for faeOS launchers. remember for durable house facts — never secrets.\n",
                "Mail: check_mail (list/hunt/read local; sync/keep/trash/send need approval). Rows have date and age — use those; the host card is now.\n",
                "WhatsApp: check_whatsapp (list/search/show/chats local; send/sync/mark_read need approval). Never invent a bash wacli. Never --follow.\n",
                "Images: generate_image hops the GPU (parks Pixie, SDXL forge, restores Pixie). Confirm names the save folder.\n",
                "Seeing: look hops to the VL eye for a picture; attached files already include an eye caption. C-u clips a file.\n",
                "World facts: search_house then wiki. No live web in Chat (that is Deep).\n",
                "If a tool returns empty or an error, say you do not know. Never invent sizes, torque, URLs, or quotes.\n",
                r#"Example: {"name": "house", "arguments": {"spell": "menagerie"}}"#,
                "\n",
                "Tool calls: EXACTLY one JSON object {\"name\": \"<tool>\", \"arguments\": {...}}."
            ),
        },
        Rune {
            key: RuneKey::Deep,
            name: "Deep",
            desc: "deep research · sources",
            rounds: 20,
            tools: vec![
                ("web_research", r#"{"query": "string", "max_results": "int"}"#),
                ("fetch_web_page", r#"{"url": "string", "max_chars": "int"}"#),
                ("wiki", r#"{"query": "string", "max_results": "int"}"#),
                ("search_house", r#"{"query": "string"}"#),
                ("read_local_file", r#"{"path": "string"}"#),
                ("list_dir", r#"{"path": "string", "max": "int"}"#),
                ("grep_files", r#"{"path": "string", "pattern": "string", "glob": "string", "max": "int"}"#),
                ("look", r#"{"path": "string", "query": "string"}"#),
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
            rounds: 32,
            tools: vec![
                ("house", r#"{"spell": "string", "args": ["string"]}"#),
                ("search_house", r#"{"query": "string"}"#),
                ("list_dir", r#"{"path": "string", "max": "int"}"#),
                ("grep_files", r#"{"path": "string", "pattern": "string", "glob": "string", "max": "int"}"#),
                ("execute_bash", r#"{"command": "string"}"#),
                ("read_local_file", r#"{"path": "string", "start_line": "int", "end_line": "int"}"#),
                ("edit_local_file", r#"{"path": "string", "old_string": "string", "new_string": "string"}"#),
                ("write_local_file", r#"{"path": "string", "content": "string"}"#),
                ("generate_image", r#"{"wish": "string"}"#),
                ("look", r#"{"path": "string", "query": "string"}"#),
            ],
            prompt: concat!(
                "You are in Build mode. Implement the ask in the named house tree (~/pixie, ~/goblin, ~/menagerie, ~/faeOS, ~/imp, …). Never list $HOME as a first step.\n",
                "1. Inspect that crate's src/ and docs: search_house, list_dir, grep_files, read_local_file. Do not bash ls/grep.\n",
                "2. edit_local_file for a few lines (old_string unique). write_local_file for new files or a full rewrite.\n",
                "3. Verify with execute_bash (cargo test --offline when a Cargo.toml exists; real shell so && works).\n",
                "4. Report: files changed, how verified. Do not write ~/.config/plan or overwrite unrelated house markdown.\n",
                "5. Self-audit rounds (default on): analysis → testing/diagnosis → docs/update notes → commit (if requested/clean) → push. Do each round explicitly, report findings, and make each round optional per user preference.\n",
                "If an active plan is in the system prompt, follow it.\n",
                "Tool calls: EXACTLY one JSON object {\"name\": \"<tool>\", \"arguments\": {...}}."
            ),
        },
        Rune {
            key: RuneKey::Plan,
            name: "Plan",
            desc: "plan first · touch nothing",
            rounds: 20,
            tools: vec![
                ("execute_bash", r#"{"command": "string"}"#),
                ("read_local_file", r#"{"path": "string", "start_line": "int", "end_line": "int"}"#),
                ("list_dir", r#"{"path": "string", "max": "int"}"#),
                ("grep_files", r#"{"path": "string", "pattern": "string", "glob": "string", "max": "int"}"#),
                ("search_house", r#"{"query": "string"}"#),
                ("wiki", r#"{"query": "string", "max_results": "int"}"#),
                ("write_plan", r#"{"goal": "string", "steps": ["string"], "files": ["string"], "risks": ["string"], "verify": "string"}"#),
                ("look", r#"{"path": "string", "query": "string"}"#),
            ],
            prompt: concat!(
                "You are in Plan mode. Design a change; do not implement it.\n",
                "1. Read the thing being planned. Pixie/Plan/Build → search_house and read_local_file ~/pixie/src/runes.rs and agent.rs. Do not list $HOME. Do not create ~/.config/plan.\n",
                "2. write_plan with goal, ordered steps, files you actually opened, risks, verify (cargo test / pixie one-shot).\n",
                "write_plan only stores ~/.cache/pixie/plans/<session>.md. Never claim you wrote ~/faeOS markdown. Never offer plan.sh.\n",
                "3. No project writes. execute_bash is read-only allowlist (ls, git status|diff|log|show|rev-parse).\n",
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
        "generate_image" => &["wish"],
        "look" => &["path"],
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
        assert_eq!(plan.rounds, 20);
        let names: Vec<_> = plan.tools.iter().map(|(n, _)| *n).collect();
        assert!(names.contains(&"write_plan"));
        assert!(names.contains(&"wiki"));
        assert!(!names.contains(&"house"));
        assert!(!names.contains(&"web_research"));
    }

    #[test]
    fn deep_has_web_and_twenty_rounds() {
        let deep = by_key(RuneKey::Deep);
        assert_eq!(deep.rounds, 20);
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
        assert_eq!(b.rounds, 32);
        assert!(b.prompt.contains("named house tree"), "{}", b.prompt);
    }

    #[test]
    fn chat_has_mail_and_whatsapp() {
        let chat = by_key(RuneKey::Chat);
        let names: Vec<_> = chat.tools.iter().map(|(n, _)| *n).collect();
        assert!(names.contains(&"check_mail"));
        assert!(names.contains(&"check_whatsapp"));
        assert!(names.contains(&"generate_image"));
        assert!(names.contains(&"look"));
        let deep = by_key(RuneKey::Deep);
        let d: Vec<_> = deep.tools.iter().map(|(n, _)| *n).collect();
        assert!(!d.contains(&"check_whatsapp"));
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
        assert!(
            h.contains("\"array\""),
            "house args should be an array: {h}"
        );
        assert!(!h.contains("web_research"), "{h}");
    }

    #[test]
    fn persona_lets_the_8b_speak() {
        assert!(PERSONA.contains("little fairy"), "{PERSONA}");
        assert!(PERSONA.contains("own voice"), "{PERSONA}");
        assert!(!PERSONA.contains("you do not"), "{PERSONA}");
        assert!(PERSONA.contains("EXACTLY one JSON object"), "{PERSONA}");
    }

    #[test]
    fn tab_cycles_four() {
        assert_eq!(next_idx(0), 1);
        assert_eq!(next_idx(3), 0);
        assert_eq!(prev_idx(0), 3);
        assert_eq!(prev_idx(1), 0);
    }
}
