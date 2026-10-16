//! Capture `<think>` blocks. Quiet hides them; jsonl always keeps them.

pub struct Split {
    pub think: String,
    pub visible: String,
}

pub fn split(text: &str) -> Split {
    let mut think = String::new();
    let mut visible = String::new();
    let mut rest = text;
    loop {
        let Some(s) = rest.to_ascii_lowercase().find("<think>") else {
            visible.push_str(rest);
            break;
        };
        visible.push_str(&rest[..s]);
        let after = &rest[s + 7..];
        let lower = after.to_ascii_lowercase();
        if let Some(e) = lower.find("</think>") {
            think.push_str(&after[..e]);
            rest = &after[e + 8..];
        } else {
            think.push_str(after);
            break;
        }
    }
    Split {
        think: think.trim().to_string(),
        visible: strip_tags(visible.trim()),
    }
}

fn strip_tags(s: &str) -> String {
    s.replace("<think>", "")
        .replace("</think>", "")
        .replace("<THINK>", "")
        .replace("</THINK>", "")
        .trim()
        .to_string()
}

pub fn strip(text: &str) -> String {
    split(text).visible
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_block() {
        let s = split("hello <think>secret plan</think> world");
        assert_eq!(s.think, "secret plan");
        assert_eq!(s.visible, "hello  world".trim() /* keep */);
        assert!(s.visible.contains("hello"));
        assert!(s.visible.contains("world"));
        assert!(!s.visible.contains("secret"));
    }

    #[test]
    fn empty_think() {
        let s = split("just a reply");
        assert!(s.think.is_empty());
        assert_eq!(s.visible, "just a reply");
    }
}
