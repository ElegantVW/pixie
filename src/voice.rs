//! SmolLM (kur :8081) writes the cute one-liner. The 8B never does sparkle.

use crate::llm;
use serde_json::json;

pub fn voice_port() -> u16 {
    std::env::var("PIXIE_VOICE_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8081)
}

const SYSTEM: &str = "You are a tiny all-ages pixie. Write ONE short remark (max 12 words) that fits the situation. No tools. No JSON. No markdown. No secrets. One line only.";

pub fn sparkle(kind: &str, seed: &str) -> String {
    if seed.trim().is_empty() {
        return String::new();
    }
    let clip: String = seed.chars().take(280).collect();
    let msgs = vec![
        json!({"role": "system", "content": SYSTEM}),
        json!({
            "role": "user",
            "content": format!("{kind}\n---\n{clip}")
        }),
    ];
    let raw = match llm::complete_on(voice_port(), &msgs, 48, 3) {
        Some(s) => s,
        None => return String::new(),
    };
    clean(&raw)
}

fn clean(s: &str) -> String {
    let mut t = s.replace('\n', " ");
    t = t.trim().trim_matches('"').to_string();
    if t.starts_with('{') || t.contains("\"name\"") {
        return String::new();
    }
    if t.chars().count() > 80 {
        t = t.chars().take(80).collect();
    }
    t
}

pub fn prepend(kind: &str, body: &str) -> String {
    let remark = sparkle(kind, body);
    if remark.is_empty() {
        return body.to_string();
    }
    format!("{remark}\n\n{body}")
}

/// Short session title from the first user prompts. Empty if kur is down.
pub fn title_thread(prompts: &[String]) -> String {
    if prompts.is_empty() {
        return String::new();
    }
    let mut seed = String::new();
    for p in prompts.iter().take(10) {
        let clip: String = p.chars().take(80).collect();
        if !seed.is_empty() {
            seed.push('\n');
        }
        seed.push_str(&clip);
    }
    let msgs = vec![
        json!({
            "role": "system",
            "content": "Name this local chat in at most 6 words. Title case. No quotes. No tools. No secrets. No Wi-Fi. One line only."
        }),
        json!({"role": "user", "content": seed}),
    ];
    let raw = match llm::complete_on_if_up(voice_port(), &msgs, 24, 3) {
        Some(s) => s,
        None => return String::new(),
    };
    let t = clean(&raw);
    if t.is_empty() || crate::mind::is_red_fact(&t) {
        return String::new();
    }
    t.chars().take(40).collect()
}

/// One-line dashboard blurb. Empty if kur is down.
pub fn summarize_thread(prompts: &[String]) -> String {
    if prompts.is_empty() {
        return String::new();
    }
    let mut seed = String::new();
    for p in prompts.iter().take(10) {
        let clip: String = p.chars().take(80).collect();
        if !seed.is_empty() {
            seed.push('\n');
        }
        seed.push_str(&clip);
    }
    let msgs = vec![
        json!({
            "role": "system",
            "content": "One short recap of this local chat, max 12 words. No quotes. No tools. No secrets. No Wi-Fi. One line only."
        }),
        json!({"role": "user", "content": seed}),
    ];
    let raw = match llm::complete_on_if_up(voice_port(), &msgs, 32, 3) {
        Some(s) => s,
        None => return String::new(),
    };
    let t = clean(&raw);
    if t.is_empty() || crate::mind::is_red_fact(&t) {
        return String::new();
    }
    t.chars().take(80).collect()
}

#[cfg(test)]
mod tests {
    use super::clean;

    #[test]
    fn clean_drops_json() {
        assert!(clean("{\"name\":\"list_dir\"}").is_empty());
        assert_eq!(clean(" wings flutter "), "wings flutter");
    }
}
