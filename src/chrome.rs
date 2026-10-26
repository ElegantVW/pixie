//! Pink boxes for one-shot replies and TUI bubbles.

use ratatui::symbols::border::{Set, PLAIN, ROUNDED};
use unicode_width::UnicodeWidthChar;

pub fn unicode() -> bool {
    match std::env::var("PIXIE_UNICODE") {
        Ok(v) => !matches!(v.as_str(), "0" | "false" | "ascii" | "no" | "off"),
        Err(_) => true,
    }
}

pub fn thread_meter(pct: u16) -> String {
    let pct = pct.min(100);
    let n = 10usize;
    let fill = (pct as usize * n) / 100;
    let (on, off) = if unicode() {
        ("█", "░")
    } else {
        ("#", "-")
    };
    format!("[{}{}] {pct}%", on.repeat(fill), off.repeat(n - fill))
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
    let tlen = str_cols(&title_s);
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
            let pad = body_w.saturating_sub(str_cols(&chunk));
            lines.push(format!("{vt} {chunk}{} {vt}", " ".repeat(pad)));
        }
    }
    if body.is_empty() {
        lines.push(format!("{vt} {} {vt}", " ".repeat(body_w)));
    }
    lines.push(format!("{bl}{}{br}", hz.to_string().repeat(inner)));
    lines.join("\n")
}

pub fn ch_cols(c: char) -> usize {
    if c == '\n' {
        0
    } else {
        UnicodeWidthChar::width(c).unwrap_or(0)
    }
}

pub fn str_cols(s: &str) -> usize {
    s.chars().map(ch_cols).sum()
}

/// Hard wrap by terminal columns (display width), not by char count.
pub fn wrap(s: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![s.to_string()];
    }
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut cur_w = 0usize;
    for ch in s.chars() {
        if ch == '\n' {
            out.push(std::mem::take(&mut cur));
            cur_w = 0;
            continue;
        }
        let w = ch_cols(ch);
        if w > 0 && cur_w + w > width && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
            cur_w = 0;
        }
        cur.push(ch);
        cur_w = cur_w.saturating_add(w);
    }
    if !cur.is_empty() || out.is_empty() {
        out.push(cur);
    }
    out
}

pub fn prompt_inner_width(term_width: u16) -> usize {
    term_width.saturating_sub(2).max(8) as usize
}

/// Outer prompt height (borders included). Floor 3, inner cap 10, never more than half the frame.
pub fn prompt_height(text: &str, term_width: u16, frame_height: u16) -> u16 {
    let inner_w = prompt_inner_width(term_width);
    let lines = wrap(text, inner_w).len().max(1) as u16;
    let half = frame_height.saturating_sub(6) / 2;
    let cap = 10u16.min(half.max(1));
    (lines.min(cap) + 2).max(3)
}

/// Row and **display column** of `char_idx` after the same wrap as `wrap`.
pub fn wrap_cursor(text: &str, width: usize, char_idx: usize) -> (u16, u16) {
    let width = width.max(1);
    let lines = wrap(text, width);
    let mut seen = 0usize;
    let last = lines.len().saturating_sub(1);
    for (i, line) in lines.iter().enumerate() {
        let n = line.chars().count();
        let end = seen + n;
        if char_idx < end || (char_idx == end && i == last) {
            let take = char_idx.saturating_sub(seen);
            let col = str_cols(&line.chars().take(take).collect::<String>());
            return (i as u16, col as u16);
        }
        seen = end;
        if text.chars().nth(seen) == Some('\n') {
            seen += 1;
        }
    }
    let col = lines.last().map(|l| str_cols(l) as u16).unwrap_or(0);
    (last as u16, col)
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
            longest = longest.max(str_cols(&chunk));
        }
    }
    ((longest + 2) as u16).clamp(min, max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thread_meter_is_ten_cells() {
        let s = thread_meter(0);
        assert!(s.contains("0%"), "{s}");
        let s = thread_meter(70);
        assert!(s.contains("70%"), "{s}");
        let s = thread_meter(100);
        assert!(s.contains("100%"), "{s}");
    }

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

    #[test]
    fn prompt_height_grows_and_caps() {
        let h = prompt_height(&"x".repeat(200), 40, 40);
        assert!(h > 3, "{h}");
        assert!(h <= 12, "{h}");
        assert_eq!(prompt_height("hi", 40, 40), 3);
    }

    #[test]
    fn wrap_cursor_tracks_char_index() {
        let (r, c) = wrap_cursor("abcdefghij", 4, 5);
        assert_eq!((r, c), (1, 1));
        let end = wrap_cursor("abcdefghij", 4, 10);
        assert_eq!(end.0, 2);
    }

    #[test]
    fn wrap_uses_columns_not_chars() {
        let star = "✦";
        let w = ch_cols('✦').max(1);
        let s = star.repeat(8);
        let lines = wrap(&s, w * 4);
        assert!(
            lines.len() >= 2,
            "expected wrap, got {lines:?} (✦ is {w} cols)"
        );
        let (row, col) = wrap_cursor(&format!("✦ {}", "x".repeat(20)), 10, 22);
        assert!(
            row >= 1,
            "cursor should leave row 0, got row={row} col={col}"
        );
    }
}
