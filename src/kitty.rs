//! Kitty graphics protocol (unicode placeholders), Magpie's framing.
//! TTY fallback stays in `forge::png_halfblocks`.

use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
};
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};

pub const PLACEHOLDER: char = '\u{10EEEE}';
const PNG_CAP: u64 = 8_000_000;

/// Kitty's unicode-placeholder row diacritics (first 40). Magpie's table.
const ROW_MARK: [u32; 40] = [
    0x0305, 0x030d, 0x030e, 0x0310, 0x0312, 0x033d, 0x033e, 0x033f, 0x0346, 0x034a,
    0x034b, 0x034c, 0x0350, 0x0351, 0x0352, 0x0357, 0x035b, 0x0363, 0x0364, 0x0365,
    0x0366, 0x0367, 0x0368, 0x0369, 0x036a, 0x036b, 0x036c, 0x036d, 0x036e, 0x036f,
    0x0483, 0x0484, 0x0485, 0x0486, 0x0487, 0x0592, 0x0593, 0x0594, 0x0595, 0x0597,
];

#[derive(Clone, Debug)]
pub struct Place {
    pub id: u32,
    pub path: PathBuf,
    pub cols: u16,
    pub rows: u16,
}

pub fn is_terminal() -> bool {
    match std::env::var("PIXIE_KITTY") {
        Ok(v) if v == "1" => return true,
        Ok(v) if v == "0" => return false,
        _ => {}
    }
    if std::env::var_os("KITTY_WINDOW_ID").is_some() {
        return true;
    }
    let term = std::env::var("TERM").unwrap_or_default().to_ascii_lowercase();
    term.contains("kitty")
}

pub fn id_for(path: &str) -> u32 {
    let mut h = 2166136261u32;
    for b in path.as_bytes() {
        h ^= u32::from(*b);
        h = h.wrapping_mul(16777619);
    }
    (h % 0xFFFFFE) + 1
}

pub fn fit_cells(width: u32, height: u32, max_cols: u16, max_rows: u16) -> (u16, u16) {
    let max_cols = max_cols.clamp(8, 96);
    let max_rows = max_rows.clamp(4, 40).min(ROW_MARK.len() as u16);
    if width == 0 || height == 0 {
        return (max_cols.min(32), max_rows.min(8));
    }
    let mut cols = max_cols;
    let mut rows = ((cols as u32 * height) / width.max(1) / 2).max(1) as u16;
    if rows > max_rows {
        rows = max_rows;
        cols = ((rows as u32 * width * 2) / height.max(1)).max(8) as u16;
    }
    (cols.clamp(8, max_cols), rows.clamp(4, max_rows))
}

pub fn fit_path(path: &Path, max_cols: u16, max_rows: u16) -> Place {
    let (w, h) = image::image_dimensions(path).unwrap_or((1024, 1024));
    let (cols, rows) = fit_cells(w, h, max_cols, max_rows);
    Place {
        id: id_for(&path.display().to_string()),
        path: path.to_path_buf(),
        cols,
        rows,
    }
}

pub fn placeholder_lines(id: u32, cols: u16, rows: u16) -> Vec<Line<'static>> {
    let cols = cols.max(1) as usize;
    let rows = rows.max(1).min(ROW_MARK.len() as u16) as usize;
    let r = ((id >> 16) & 0xFF) as u8;
    let g = ((id >> 8) & 0xFF) as u8;
    let b = (id & 0xFF) as u8;
    let style = Style::default().fg(Color::Rgb(r, g, b));
    let mut out = Vec::with_capacity(rows);
    for row in 0..rows {
        let mark = char::from_u32(ROW_MARK[row]).unwrap_or('\u{0305}');
        let mut s = String::with_capacity(cols * 2);
        s.push(PLACEHOLDER);
        s.push(mark);
        for _ in 1..cols {
            s.push(PLACEHOLDER);
        }
        out.push(Line::from(Span::styled(s, style)));
    }
    out
}

pub fn transmit(place: &Place) -> Vec<u8> {
    let Ok(bytes) = std::fs::read(&place.path) else {
        return Vec::new();
    };
    let (fmt, payload) = if bytes.len() as u64 <= PNG_CAP
        && bytes.starts_with(b"\x89PNG")
    {
        (100u16, bytes)
    } else {
        let Ok(img) = image::load_from_memory(&bytes) else {
            return Vec::new();
        };
        let rgb = img.to_rgb8();
        let mut raw = Vec::with_capacity(rgb.len());
        raw.extend_from_slice(&rgb.into_raw());
        return transmit_rgb(place, img.width(), img.height(), &raw);
    };
    let _ = fmt;
    transmit_png(place, &payload)
}

fn transmit_png(place: &Place, png: &[u8]) -> Vec<u8> {
    let b64 = b64_encode(png);
    let head = format!(
        "a=T,f=100,i={},q=2,U=1,c={},r={},",
        place.id, place.cols, place.rows
    );
    chunked_apc(&head, &b64)
}

fn transmit_rgb(place: &Place, width: u32, height: u32, rgb: &[u8]) -> Vec<u8> {
    let b64 = b64_encode(rgb);
    let head = format!(
        "a=T,f=24,s={width},v={height},i={},q=2,U=1,c={},r={},",
        place.id, place.cols, place.rows
    );
    chunked_apc(&head, &b64)
}

fn chunked_apc(head: &str, b64: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let mut first = true;
    let mut i = 0;
    while i < b64.len() {
        let end = (i + 4096).min(b64.len());
        let last = end == b64.len();
        let control = if first {
            format!("{head}m={}", if last { 0 } else { 1 })
        } else {
            format!("m={}", if last { 0 } else { 1 })
        };
        out.extend_from_slice(b"\x1b_G");
        out.extend_from_slice(control.as_bytes());
        out.push(b';');
        out.extend_from_slice(&b64.as_bytes()[i..end]);
        out.extend_from_slice(b"\x1b\\");
        first = false;
        i = end;
    }
    out
}

pub fn delete(id: u32) -> Vec<u8> {
    format!("\x1b_Ga=d,d=I,i={id},q=2\x1b\\").into_bytes()
}

/// Transmit new placements; delete ids that left the frame.
pub fn sync(
    out: &mut impl Write,
    frame: &HashMap<u32, Place>,
    sent: &mut HashMap<u32, (u16, u16)>,
) {
    let gone: Vec<u32> = sent
        .keys()
        .copied()
        .filter(|id| !frame.contains_key(id))
        .collect();
    for id in gone {
        let _ = out.write_all(&delete(id));
        sent.remove(&id);
    }
    for (id, place) in frame {
        let size = (place.cols, place.rows);
        if sent.get(id) == Some(&size) {
            continue;
        }
        if sent.contains_key(id) {
            let _ = out.write_all(&delete(*id));
        }
        let payload = transmit(place);
        if !payload.is_empty() {
            let _ = out.write_all(&payload);
            sent.insert(*id, size);
        }
    }
    let _ = out.flush();
}

pub fn clear(out: &mut impl Write, sent: &mut HashMap<u32, (u16, u16)>) {
    for id in sent.keys().copied().collect::<Vec<_>>() {
        let _ = out.write_all(&delete(id));
    }
    sent.clear();
    let _ = out.flush();
}

pub(crate) fn b64_encode(data: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    let mut i = 0;
    while i + 3 <= data.len() {
        let n = ((data[i] as u32) << 16) | ((data[i + 1] as u32) << 8) | data[i + 2] as u32;
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        out.push(T[((n >> 6) & 63) as usize] as char);
        out.push(T[(n & 63) as usize] as char);
        i += 3;
    }
    if i < data.len() {
        let a = data[i];
        let b = data.get(i + 1).copied().unwrap_or(0);
        let n = ((a as u32) << 16) | ((b as u32) << 8);
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        if i + 1 < data.len() {
            out.push(T[((n >> 6) & 63) as usize] as char);
            out.push('=');
        } else {
            out.push('=');
            out.push('=');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholder_uses_kitty_char() {
        let lines = placeholder_lines(42, 4, 2);
        assert_eq!(lines.len(), 2);
        for line in &lines {
            let s = line.to_string();
            assert!(s.contains(PLACEHOLDER), "{s:?}");
        }
    }

    #[test]
    fn transmit_png_header() {
        let png = {
            let mut v = Vec::new();
            let img = image::RgbImage::from_pixel(2, 2, image::Rgb([0xff, 0, 0]));
            image::DynamicImage::ImageRgb8(img)
                .write_to(&mut std::io::Cursor::new(&mut v), image::ImageFormat::Png)
                .unwrap();
            v
        };
        let dir = std::env::temp_dir().join(format!("pixie-kitty-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join("tiny.png");
        std::fs::write(&p, &png).unwrap();
        let place = Place {
            id: 7,
            path: p,
            cols: 4,
            rows: 2,
        };
        let seq = transmit(&place);
        let s = String::from_utf8_lossy(&seq);
        assert!(s.contains("\u{1b}_Ga=T,f=100,i=7"), "{s}");
        assert!(seq.ends_with(b"\x1b\\"), "{seq:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fit_keeps_aspect_under_cap() {
        let (c, r) = fit_cells(1024, 1024, 72, 24);
        assert!(c <= 72 && r <= 24, "{c}x{r}");
        assert!(c >= 8 && r >= 4, "{c}x{r}");
    }

    #[test]
    fn id_is_stable_and_nonzero() {
        let a = id_for("/tmp/a.png");
        assert_eq!(a, id_for("/tmp/a.png"));
        assert_ne!(a, 0);
        assert_ne!(a, id_for("/tmp/b.png"));
    }
}
