//! Pink boxes for one-shot replies and TUI bubbles.

use ratatui::symbols::border::{Set, PLAIN, ROUNDED};

pub fn unicode() -> bool {
    match std::env::var("PIXIE_UNICODE") {
        Ok(v) => !matches!(v.as_str(), "0" | "false" | "ascii" | "no" | "off"),
        Err(_) => true,
    }
}

pub fn border_set() -> Set {
    if unicode() {
        ROUNDED
    } else {
        PLAIN
    }
}

pub fn box_text(title: &str, body: &str, width: usize) -> String {
    let w = width.max(24);
    let inner = w.saturating_sub(2);
    let (tl, tr, bl, br, hz, vt, mark) = if unicode() {
        ('╭', '╮', '╰', '╯', '─', '│', '✦')
    } else {
        ('+', '+', '+', '+', '-', '|', '*')
    };
    let mut title_s = format!(" {mark} {title} {mark} ");
    if title_s.chars().count() > inner {
        title_s = title_s.chars().take(inner).collect();
    }
    let tlen = title_s.chars().count();
    let fill = inner.saturating_sub(tlen);
    let left = fill / 2;
    let right = fill - left;
    let mut lines = vec![format!(
        "{tl}{}{title_s}{}{tr}",
        hz.to_string().repeat(left),
        hz.to_string().repeat(right)
    )];
    let body_w = inner.saturating_sub(2);
    for raw in body.lines() {
        for chunk in wrap(raw, body_w) {
            let pad = body_w.saturating_sub(chunk.chars().count());
            lines.push(format!("{vt} {chunk}{} {vt}", " ".repeat(pad)));
        }
    }
    if body.is_empty() {
        lines.push(format!("{vt} {} {vt}", " ".repeat(body_w)));
    }
    lines.push(format!("{bl}{}{br}", hz.to_string().repeat(inner)));
    lines.join("\n")
}

pub fn wrap(s: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![s.to_string()];
    }
    let mut out = Vec::new();
    let mut cur = String::new();
    for ch in s.chars() {
        if ch == '\n' {
            out.push(std::mem::take(&mut cur));
            continue;
        }
        if cur.chars().count() >= width {
            out.push(std::mem::take(&mut cur));
        }
        cur.push(ch);
    }
    if !cur.is_empty() || out.is_empty() {
        out.push(cur);
    }
    out
}

pub fn term_width() -> usize {
    crossterm::terminal::size()
        .map(|(w, _)| w as usize)
        .unwrap_or(76)
        .clamp(40, 76)
}

/// Content-sized bubble outer width: longest wrapped line + borders, clamped.
pub fn bubble_width(text: &str, max: u16, min: u16) -> u16 {
    let max = max.max(min);
    let inner_max = max.saturating_sub(2) as usize;
    let mut longest = 0usize;
    if text.is_empty() {
        longest = 1;
    }
    for raw in text.lines() {
        for chunk in wrap(raw, inner_max.max(1)) {
            longest = longest.max(chunk.chars().count());
        }
    }
    ((longest + 2) as u16).clamp(min, max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_splits_on_width() {
        let got = wrap("abcdefghij", 4);
        assert_eq!(got, vec!["abcd", "efgh", "ij"]);
    }

    #[test]
    fn bubble_width_clamps() {
        let w = bubble_width("hi", 40, 12);
        assert_eq!(w, 12);
        let w = bubble_width(&"x".repeat(80), 40, 12);
        assert_eq!(w, 40);
    }

    #[test]
    fn box_text_has_title() {
        let s = box_text("Pixie", "hello", 40);
        assert!(s.contains("Pixie"), "{s}");
        assert!(s.contains("hello"), "{s}");
    }
}
