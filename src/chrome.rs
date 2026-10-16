//! Pink unicode boxes for one-shot replies.

pub fn box_text(title: &str, body: &str, width: usize) -> String {
    let w = width.max(24);
    let inner = w.saturating_sub(2);
    let mut title_s = format!(" ✦ {title} ✦ ");
    if title_s.chars().count() > inner {
        title_s = title_s.chars().take(inner).collect();
    }
    let tlen = title_s.chars().count();
    let fill = inner.saturating_sub(tlen);
    let left = fill / 2;
    let right = fill - left;
    let mut lines = vec![format!(
        "╭{}{title_s}{}╮",
        "─".repeat(left),
        "─".repeat(right)
    )];
    for raw in body.lines() {
        for chunk in wrap(raw, inner.saturating_sub(2)) {
            let pad = inner.saturating_sub(2).saturating_sub(chunk.chars().count());
            lines.push(format!("│ {chunk}{} │", " ".repeat(pad)));
        }
    }
    if body.is_empty() {
        lines.push(format!("│ {} │", " ".repeat(inner.saturating_sub(2))));
    }
    lines.push(format!("╰{}╯", "─".repeat(inner)));
    lines.join("\n")
}

fn wrap(s: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![s.to_string()];
    }
    let mut out = Vec::new();
    let mut cur = String::new();
    for ch in s.chars() {
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
