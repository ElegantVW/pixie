//! Spoken tongue. `voice.rs` stays SmolLM sparkle text on kur :8081.

use crate::mind;
use crate::paths;
use serde_json::json;
use std::fs;
use std::io::Write;
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

pub const PORT: u16 = 8084;
pub const CLIP_CAP: usize = 800;

/// Physical cores minus two, so the box can still breathe. SMT counts as two threads per core.
pub fn max_threads() -> u16 {
    let n = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(8);
    let cores = (n / 2).max(1);
    cores.saturating_sub(2).max(2) as u16
}

/// Short clips do not wake every core. Sidecar stays at `max_threads`; oneshot scales.
pub fn thread_count(clip_chars: usize) -> u16 {
    let cap = max_threads();
    if clip_chars <= 48 {
        6.min(cap).max(2)
    } else if clip_chars <= 160 {
        10.min(cap).max(4)
    } else {
        cap
    }
}

const CODEC_NAME: &str = "qwen3-tts-tokenizer-12hz.gguf";
const VIVIAN_TALKER: &str = "qwen3-tts-12hz-0.6b-customvoice-q8_0.gguf";
const DESIGN_TALKER: &str = "qwen3-tts-12hz-1.7b-voicedesign-q8_0.gguf";

const VIVIAN_INSTRUCT: &str =
    "Bright small all-ages fairy creature, quick and kind, high and light, not a child.";
const DESIGN_INSTRUCT: &str = "Tiny all-ages fairy creature, not a child and not a human woman. High bell-like voice, quick and kind, a little sparkle, English.";

/// Default Pixie voice. VoiceDesign is the second tongue.
pub const SPEAKER: &str = "vivian";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tongue {
    Vivian,
    Design,
}

impl Tongue {
    pub fn from_str(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "vivian" | "custom" | "customvoice" => Tongue::Vivian,
            _ => Tongue::Design,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Tongue::Vivian => "vivian",
            Tongue::Design => "design",
        }
    }

    pub fn backend(self) -> &'static str {
        match self {
            Tongue::Vivian => "qwen3-tts-customvoice",
            Tongue::Design => "qwen3-tts-1.7b-voicedesign",
        }
    }

    pub fn talker_name(self) -> &'static str {
        match self {
            Tongue::Vivian => VIVIAN_TALKER,
            Tongue::Design => DESIGN_TALKER,
        }
    }

    pub fn instruct(self) -> &'static str {
        match self {
            Tongue::Vivian => VIVIAN_INSTRUCT,
            Tongue::Design => DESIGN_INSTRUCT,
        }
    }

    pub fn voice(self) -> Option<&'static str> {
        match self {
            Tongue::Vivian => Some("vivian"),
            Tongue::Design => None,
        }
    }

    pub fn next(self) -> Self {
        match self {
            Tongue::Vivian => Tongue::Design,
            Tongue::Design => Tongue::Vivian,
        }
    }
}

struct Sidecar {
    child: Option<Child>,
    tongue: Option<Tongue>,
}

fn sidecar_cell() -> &'static Mutex<Sidecar> {
    static C: OnceLock<Mutex<Sidecar>> = OnceLock::new();
    C.get_or_init(|| {
        Mutex::new(Sidecar {
            child: None,
            tongue: None,
        })
    })
}

pub fn bin() -> PathBuf {
    paths::home().join(".local/lib/crispasr/crispasr")
}

pub fn lib_dir() -> PathBuf {
    paths::home().join(".local/lib/crispasr")
}

pub fn talker() -> PathBuf {
    talker_for(Tongue::Vivian)
}

pub fn talker_for(tongue: Tongue) -> PathBuf {
    paths::data_dir().join("models").join(tongue.talker_name())
}

pub fn codec() -> PathBuf {
    paths::data_dir().join("models").join(CODEC_NAME)
}

pub fn cache_wav() -> PathBuf {
    paths::cache_dir().join("speak").join("last.wav")
}

fn sidecar_log() -> PathBuf {
    paths::cache_dir().join("speak").join("sidecar.log")
}

const SENTENCE_FLOOR: usize = 12;
const FIRST_CLAUSE_FLOOR: usize = 24;
const WAV_RATE: u32 = 24_000;

/// Pull complete sentences off `buf`. Remainder has no terminal `.?!` yet, or is under the floor.
pub fn take_sentences(buf: &str) -> (Vec<String>, String) {
    take_speech_units(buf, false)
}

/// First breath of a turn: any `.?!` goes, and a long first clause may split on `,;:`.
pub fn take_speech_units(buf: &str, first: bool) -> (Vec<String>, String) {
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut i = 0usize;
    while i < buf.len() {
        let ch = match buf[i..].chars().next() {
            Some(c) => c,
            None => break,
        };
        let n = ch.len_utf8();
        let stop = matches!(ch, '.' | '!' | '?');
        let clause = first && out.is_empty() && matches!(ch, ',' | ';' | ':');
        if stop || clause {
            let end = i + n;
            if clause {
                let next = buf[end..].chars().next();
                if next.map(|c| c.is_ascii_digit()).unwrap_or(false) {
                    i += n;
                    continue;
                }
            }
            let piece = buf[start..end].trim();
            let need = if stop {
                if first && out.is_empty() {
                    1
                } else {
                    SENTENCE_FLOOR
                }
            } else {
                FIRST_CLAUSE_FLOOR
            };
            if piece.chars().count() >= need {
                out.push(piece.to_string());
                start = end;
            }
        }
        i += n;
    }
    (out, buf[start..].to_string())
}

enum Job {
    Begin { gen: u64 },
    Line { gen: u64, tongue: Tongue, text: String },
    Flush { gen: u64 },
}

enum PlayJob {
    Begin { gen: u64 },
    Clip { gen: u64, text: String, path: PathBuf },
    Flush { gen: u64 },
}

/// Play-worker → TUI. Glass paints `Line` when that sentence starts playing.
#[derive(Debug, Clone)]
pub enum HearEvent {
    Line { gen: u64, text: String },
    Caught { gen: u64 },
}

fn hear_cell() -> &'static Mutex<Option<std::sync::mpsc::Sender<HearEvent>>> {
    static C: OnceLock<Mutex<Option<std::sync::mpsc::Sender<HearEvent>>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(None))
}

pub fn set_hear(tx: std::sync::mpsc::Sender<HearEvent>) {
    if let Ok(mut g) = hear_cell().lock() {
        *g = Some(tx);
    }
}

fn emit_hear(ev: HearEvent) {
    if let Ok(g) = hear_cell().lock() {
        if let Some(tx) = g.as_ref() {
            let _ = tx.send(ev);
        }
    }
}

struct Streamer {
    last_clip: String,
    remainder: String,
    emitted: u32,
    gen: u64,
}

fn streamer() -> &'static Mutex<Streamer> {
    static C: OnceLock<Mutex<Streamer>> = OnceLock::new();
    C.get_or_init(|| {
        Mutex::new(Streamer {
            last_clip: String::new(),
            remainder: String::new(),
            emitted: 0,
            gen: 0,
        })
    })
}

fn jobs() -> Option<std::sync::mpsc::Sender<Job>> {
    if cfg!(test) {
        return None;
    }
    static TX: OnceLock<std::sync::mpsc::Sender<Job>> = OnceLock::new();
    Some(
        TX.get_or_init(|| {
            let (tx, rx) = std::sync::mpsc::channel();
            let (play_tx, play_rx) = std::sync::mpsc::channel::<PlayJob>();
            let _ = std::thread::Builder::new()
                .name("pixie-tongue-play".into())
                .spawn(move || play_worker(play_rx));
            let _ = std::thread::Builder::new()
                .name("pixie-tongue-synth".into())
                .spawn(move || synth_worker(rx, play_tx));
            tx
        })
        .clone(),
    )
}

fn synth_worker(rx: std::sync::mpsc::Receiver<Job>, play_tx: std::sync::mpsc::Sender<PlayJob>) {
    let mut n = 0u32;
    let mut gen = 0u64;
    while let Ok(job) = rx.recv() {
        match job {
            Job::Begin { gen: g } => {
                gen = g;
                n = 0;
                let _ = play_tx.send(PlayJob::Begin { gen: g });
            }
            Job::Line {
                gen: g,
                tongue,
                text,
            } => {
                if g != gen {
                    continue;
                }
                let clip = spoken_plain(&text);
                if clip.is_empty() {
                    continue;
                }
                n = n.saturating_add(1);
                let dest = paths::cache_dir()
                    .join("speak")
                    .join(format!("part-{n:03}.wav"));
                let _ = fs::create_dir_all(dest.parent().unwrap_or(Path::new(".")));
                if synth_to(tongue, &clip, &dest).is_ok() {
                    let _ = play_tx.send(PlayJob::Clip {
                        gen: g,
                        text,
                        path: dest,
                    });
                }
            }
            Job::Flush { gen: g } => {
                if g == gen {
                    let _ = play_tx.send(PlayJob::Flush { gen: g });
                }
            }
        }
    }
}

fn play_worker(rx: std::sync::mpsc::Receiver<PlayJob>) {
    let mut pcm = Vec::new();
    while let Ok(job) = rx.recv() {
        match job {
            PlayJob::Begin { .. } => {
                pcm.clear();
            }
            PlayJob::Clip { gen, text, path } => {
                emit_hear(HearEvent::Line { gen, text });
                let _ = play_wav(&path);
                append_wav_pcm(&mut pcm, &path);
                let _ = write_pcm_wav(&pcm, &cache_wav());
            }
            PlayJob::Flush { gen } => {
                emit_hear(HearEvent::Caught { gen });
            }
        }
    }
}

fn append_wav_pcm(pcm: &mut Vec<u8>, wav: &Path) {
    let Ok(b) = fs::read(wav) else {
        return;
    };
    if b.len() > 44 && b.starts_with(b"RIFF") {
        pcm.extend_from_slice(&b[44..]);
    }
}

fn write_pcm_wav(pcm: &[u8], dest: &Path) -> Result<(), String> {
    let mut out = Vec::with_capacity(44 + pcm.len());
    let data_len = pcm.len() as u32;
    let riff_len = 36u32 + data_len;
    let byte_rate = WAV_RATE * 2;
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&riff_len.to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&WAV_RATE.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    out.extend_from_slice(pcm);
    fs::write(dest, out).map_err(|e| format!("pixie: tongue wav: {e}"))
}

pub fn begin_turn() -> u64 {
    if cfg!(test) {
        return 0;
    }
    let gen = if let Ok(mut g) = streamer().lock() {
        g.last_clip.clear();
        g.remainder.clear();
        g.emitted = 0;
        g.gen = g.gen.saturating_add(1);
        g.gen
    } else {
        1
    };
    if let Some(tx) = jobs() {
        let _ = tx.send(Job::Begin { gen });
    }
    gen
}

pub fn drop_partial() {
    if cfg!(test) {
        return;
    }
    if let Ok(mut g) = streamer().lock() {
        g.last_clip.clear();
        g.remainder.clear();
    }
}

pub fn emitted_this_turn() -> bool {
    streamer()
        .lock()
        .map(|g| g.emitted > 0 || !g.remainder.trim().is_empty())
        .unwrap_or(false)
}

pub fn push_live(tongue: Tongue, visible: &str) {
    if cfg!(test) {
        return;
    }
    let clip = spoken_clip(visible);
    if clip.is_empty() {
        return;
    }
    let mut sents = Vec::new();
    if let Ok(mut g) = streamer().lock() {
        let extra = if clip.starts_with(&g.last_clip) {
            clip[g.last_clip.len()..].to_string()
        } else if g.last_clip.starts_with(&clip) {
            String::new()
        } else {
            g.remainder.clear();
            clip.clone()
        };
        g.last_clip = clip;
        if extra.is_empty() {
            return;
        }
        g.remainder.push_str(&extra);
        let first = g.emitted == 0;
        let (got, rest) = take_speech_units(&g.remainder, first);
        g.remainder = rest;
        g.emitted = g.emitted.saturating_add(got.len() as u32);
        sents = got;
    }
    for s in sents {
        enqueue(tongue, s);
    }
}

pub fn flush_remainder(tongue: Tongue) {
    if cfg!(test) {
        return;
    }
    let rest = streamer()
        .lock()
        .ok()
        .map(|mut g| {
            let r = std::mem::take(&mut g.remainder);
            if !r.trim().is_empty() {
                g.emitted = g.emitted.saturating_add(1);
            }
            r
        })
        .unwrap_or_default();
    let rest = rest.trim();
    if !rest.is_empty() {
        enqueue(tongue, rest.to_string());
    }
    let gen = streamer().lock().map(|g| g.gen).unwrap_or(0);
    if let Some(tx) = jobs() {
        let _ = tx.send(Job::Flush { gen });
    }
}

fn enqueue(tongue: Tongue, text: String) {
    if text.trim().is_empty() {
        return;
    }
    let gen = streamer().lock().map(|g| g.gen).unwrap_or(0);
    if let Some(tx) = jobs() {
        let _ = tx.send(Job::Line { gen, tongue, text });
    }
}

/// Synth a clip to `dest` without playing. Tests are no-ops.
pub fn synth_to(tongue: Tongue, text: &str, dest: &Path) -> Result<(), String> {
    if cfg!(test) {
        return Ok(());
    }
    let clip = spoken_plain(text);
    if clip.is_empty() {
        return Ok(());
    }
    if !ready_tongue(tongue) {
        return Err(sleeping_reason_tongue(tongue));
    }
    if let Some(dir) = dest.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("pixie: tongue cache: {e}"))?;
    }
    let _ = fs::remove_file(dest);
    match ensure_tongue(tongue) {
        Ok(()) => {
            if synth_http(tongue, &clip, dest).is_err() {
                synth_oneshot(tongue, &clip, dest)?;
            }
        }
        Err(_) => synth_oneshot(tongue, &clip, dest)?,
    }
    Ok(())
}

fn file_ok(p: &Path, min: u64) -> bool {
    p.is_file() && fs::metadata(p).map(|m| m.len()).unwrap_or(0) >= min
}

pub fn ready_from(bin: &Path, talker: &Path, codec: &Path) -> bool {
    file_ok(bin, 1_000_000) && file_ok(talker, 1_000_000) && file_ok(codec, 1_000_000)
}

pub fn ready_tongue(tongue: Tongue) -> bool {
    ready_from(&bin(), &talker_for(tongue), &codec())
}

pub fn ready() -> bool {
    ready_tongue(Tongue::Vivian)
}

pub fn sleeping_reason_from(bin: &Path, talker: &Path, codec: &Path) -> String {
    if !file_ok(bin, 1_000_000) {
        return "tongue is still sleeping · next: park ~/.local/lib/crispasr/crispasr".into();
    }
    if !file_ok(talker, 1_000_000) {
        return format!(
            "tongue is still sleeping · next: park {}",
            talker.display()
        );
    }
    if !file_ok(codec, 1_000_000) {
        return format!(
            "tongue is still sleeping · next: park {}",
            codec.display()
        );
    }
    "tongue is still sleeping".into()
}

pub fn sleeping_reason() -> String {
    sleeping_reason_tongue(Tongue::Vivian)
}

pub fn sleeping_reason_tongue(tongue: Tongue) -> String {
    sleeping_reason_from(&bin(), &talker_for(tongue), &codec())
}

fn looks_like_tool(t: &str) -> bool {
    let s = t.trim();
    if s.is_empty() {
        return false;
    }
    if s.starts_with('{') || s.contains("<tool_call>") || s.starts_with("```json") {
        return true;
    }
    s.contains("\"name\"") && s.contains("\"arguments\"")
}

/// Final assistant prose only. Drops tool JSON, red facts, and a wall of text.
pub fn spoken_clip(text: &str) -> String {
    let t = text.trim();
    if t.is_empty() || mind::is_red_fact(t) || looks_like_tool(t) {
        return String::new();
    }
    let mut out = String::new();
    for w in t.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(w);
        if out.chars().count() >= CLIP_CAP {
            break;
        }
    }
    if out.chars().count() > CLIP_CAP {
        let clipped: String = out.chars().take(CLIP_CAP).collect();
        if let Some(i) = clipped.rfind(['.', '!', '?']) {
            return clipped[..=i].trim().to_string();
        }
        return clipped.trim().to_string();
    }
    out
}

/// What the talker says. Glass keeps the raw reply.
pub fn spoken_plain(text: &str) -> String {
    let clip = spoken_clip(text);
    if clip.is_empty() {
        return String::new();
    }
    let cleaned = strip_technical(&clip);
    let glossed = gloss_emoji(&cleaned);
    collapse_ws(&glossed)
}

fn collapse_ws(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut gap = false;
    for ch in s.chars() {
        if ch.is_whitespace() {
            gap = true;
            continue;
        }
        if gap && !out.is_empty() {
            out.push(' ');
        }
        gap = false;
        out.push(ch);
    }
    out.trim().to_string()
}

fn strip_technical(s: &str) -> String {
    let s = strip_fences(s);
    let s = strip_inline_code(&s);
    let s = strip_md_media(&s);
    let s = strip_urls(&s);
    let s = strip_html(&s);
    let s = strip_emphasis(&s);
    strip_path_tokens(&s)
}

fn strip_fences(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("```") {
        out.push_str(&rest[..i]);
        rest = &rest[i + 3..];
        if let Some(nl) = rest.find('\n') {
            rest = &rest[nl + 1..];
        }
        if let Some(j) = rest.find("```") {
            rest = &rest[j + 3..];
        } else {
            rest = "";
            break;
        }
    }
    out.push_str(rest);
    out
}

fn strip_inline_code(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('`') {
        out.push_str(&rest[..i]);
        rest = &rest[i + 1..];
        if let Some(j) = rest.find('`') {
            let inner = &rest[..j];
            if inner.chars().all(|c| c.is_alphanumeric() || c == ' ' || c == '-') {
                out.push_str(inner);
            }
            rest = &rest[j + 1..];
        } else {
            break;
        }
    }
    out.push_str(rest);
    out
}

fn strip_md_media(s: &str) -> String {
    let mut out = String::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0usize;
    while i < chars.len() {
        if chars[i] == '!' && i + 1 < chars.len() && chars[i + 1] == '[' {
            if let Some(end) = skip_md_link(&chars, i + 1) {
                i = end;
                continue;
            }
        }
        if chars[i] == '[' {
            if let Some((text, end)) = take_md_link(&chars, i) {
                out.push_str(&text);
                i = end;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn skip_md_link(chars: &[char], open: usize) -> Option<usize> {
    let ( _, end) = take_md_link(chars, open)?;
    Some(end)
}

fn take_md_link(chars: &[char], open: usize) -> Option<(String, usize)> {
    if open >= chars.len() || chars[open] != '[' {
        return None;
    }
    let mut i = open + 1;
    let mut text = String::new();
    while i < chars.len() && chars[i] != ']' {
        text.push(chars[i]);
        i += 1;
        if text.len() > 200 {
            return None;
        }
    }
    if i + 1 >= chars.len() || chars[i] != ']' || chars[i + 1] != '(' {
        return None;
    }
    i += 2;
    let mut depth = 1i32;
    while i < chars.len() {
        if chars[i] == '(' {
            depth += 1;
        } else if chars[i] == ')' {
            depth -= 1;
            if depth == 0 {
                return Some((text, i + 1));
            }
        }
        i += 1;
        if i - open > 400 {
            return None;
        }
    }
    None
}

fn strip_urls(s: &str) -> String {
    let mut out = String::new();
    for w in s.split_whitespace() {
        let t = w.trim_matches(|c: char| "<>()[]{},;\"'".contains(c));
        let low = t.to_ascii_lowercase();
        if low.starts_with("http://")
            || low.starts_with("https://")
            || low.starts_with("ftp://")
            || low.starts_with("file://")
            || low.starts_with("www.")
        {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(w);
    }
    out
}

fn strip_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for ch in s.chars() {
        if ch == '<' {
            in_tag = true;
            continue;
        }
        if ch == '>' && in_tag {
            in_tag = false;
            continue;
        }
        if !in_tag {
            out.push(ch);
        }
    }
    out
}

fn strip_emphasis(s: &str) -> String {
    s.replace("**", "")
        .replace("__", "")
        .replace("~~", "")
        .replace('*', "")
        .replace('_', " ")
}

fn strip_path_tokens(s: &str) -> String {
    let mut out = String::new();
    for w in s.split_whitespace() {
        if looks_like_path(w) {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(w);
    }
    out
}

fn looks_like_path(w: &str) -> bool {
    let t = w.trim_matches(|c: char| "()[]{},;:\"'".contains(c));
    if t.starts_with("~/") || t.starts_with("/home/") || t.starts_with("/usr/") || t.starts_with("/etc/")
        || t.starts_with("/var/") || t.starts_with("/tmp/") || t.starts_with("/opt/")
        || t.starts_with("/dev/")
    {
        return true;
    }
    let low = t.to_ascii_lowercase();
    if low.ends_with(".gguf")
        || low.ends_with(".wav")
        || low.ends_with(".ogg")
        || low.ends_with(".opus")
        || low.ends_with(".json")
        || low.ends_with(".jsonl")
        || low.ends_with(".toml")
        || low.ends_with(".lock")
    {
        return t.contains('/') || t.contains('\\');
    }
    t.matches('/').count() >= 2 && t.contains('.')
}

fn gloss_emoji(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        if is_cluster_cont(c) {
            i += 1;
            continue;
        }
        if (0x1F1E6..=0x1F1FF).contains(&(c as u32)) {
            i += 1;
            if i < chars.len() && (0x1F1E6..=0x1F1FF).contains(&(chars[i] as u32)) {
                i += 1;
            }
            continue;
        }
        if let Some(g) = emoji_gloss(c) {
            i += 1;
            while i < chars.len() && is_cluster_cont(chars[i]) {
                i += 1;
            }
            if !g.is_empty() {
                if !out.is_empty() && !out.ends_with(' ') {
                    out.push(' ');
                }
                out.push_str(g);
                out.push(' ');
            }
            continue;
        }
        if is_emoji_char(c) {
            i += 1;
            while i < chars.len() && is_cluster_cont(chars[i]) {
                i += 1;
            }
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

fn is_cluster_cont(c: char) -> bool {
    matches!(
        c as u32,
        0x200D | 0xFE0E | 0xFE0F | 0x20E3 | 0x1F3FB..=0x1F3FF
    )
}

fn is_emoji_char(c: char) -> bool {
    let u = c as u32;
    matches!(
        u,
        0x2190..=0x21FF
            | 0x2300..=0x23FF
            | 0x25A0..=0x25FF
            | 0x2600..=0x27BF
            | 0x2B00..=0x2BFF
            | 0x1F000..=0x1FAFF
    )
}

/// Spoken gloss for a lead emoji. Empty string = drop. None = not an emoji we map.
fn emoji_gloss(c: char) -> Option<&'static str> {
    Some(match c {
        '😀' | '😃' | '😄' | '😁' | '😆' | '😅' | '😂' | '🤣' | '😹' | '😸' | '😺' | '🙃'
        | '🫠' => "ha ha",
        '🤭' | '😏' | '😼' | '😜' | '😝' | '😛' | '🤪' => "heh",
        '🤔' | '🧐' | '🤨' => "hmm",
        '😴' | '😪' | '🤤' | '🥱' | '💤' => "yawn",
        '🤧' => "achoo",
        '😱' | '😨' | '😰' => "ah",
        '😮' | '😲' | '😯' | '😦' | '😧' => "oh",
        '🥳' | '🎉' | '🎊' | '🎈' => "yay",
        '😢' | '😭' | '😿' | '😞' | '😔' | '😟' | '🙁' | '☹' | '🥺' | '🥹' => "aw",
        '👏' => "clap",
        '🎵' | '🎶' | '🎤' => "hum",
        '🤫' => "shh",
        '😤' | '😠' | '😡' | '🤬' => "hmph",
        '😇' | '😊' | '🙂' | '☺' | '🥰' | '😍' | '🤩' | '😘' | '😗' | '😚' | '😙' | '🤗' => "",
        '❤' | '💕' | '💖' | '💗' | '💓' | '💞' | '💘' | '💝' | '💟' | '🧡' | '💛' | '💚'
        | '💙' | '💜' | '🖤' | '🤍' | '🤎' | '💔' | '💋' => "",
        '✨' | '⭐' | '🌟' | '💫' | '⚡' | '🔥' | '💯' => "",
        '👍' | '👎' | '👌' | '✌' | '🤞' | '🤟' | '🤘' | '👋' | '🤚' | '✋' | '🖐' | '🙏'
        | '🙌' | '💪' | '🤷' | '🤦' => "",
        '👀' | '👁' | '🧠' | '💀' | '☠' | '👻' | '🤖' | '👽' | '💩' | '😈' | '👿' => "",
        '🧚' | '🧙' | '🧛' | '🧜' | '🧝' | '🪄' => "",
        '🌙' | '☀' | '🌞' | '🌝' | '🌸' | '🌺' | '🌹' | '🌷' | '🌼' | '🌈' | '🦄' | '🐶'
        | '🐱' => "",
        '🍕' | '🍔' | '☕' | '⏰' | '🎁' | '🎂' => "",
        '📱' | '📞' | '💻' | '🖥' | '⌨' | '📧' | '📨' | '📩' => "",
        '✅' | '✔' | '☑' | '❌' | '❎' | '✖' | '⚠' | '⛔' | '🚫' | '❓' | '❔' | '❗' | '❕' => "",
        '📌' | '📍' | '🔗' | '📁' | '📂' | '📄' | '📝' | '🔒' | '🔓' | '🔑' | '⚙' | '🔧'
        | '🛠' | '🚀' | '💰' | '💸' | '💬' | '🫡' | '🫣' => "",
        _ => return None,
    })
}

fn push_common(
    argv: &mut Vec<String>,
    bin: &Path,
    tongue: Tongue,
    talker: &Path,
    codec: &Path,
    threads: u16,
) {
    argv.push(bin.display().to_string());
    argv.push("--no-gpu".into());
    argv.push("-t".into());
    argv.push(threads.to_string());
    argv.push("--backend".into());
    argv.push(tongue.backend().into());
    argv.push("-m".into());
    argv.push(talker.display().to_string());
    argv.push("--codec-model".into());
    argv.push(codec.display().to_string());
    if let Some(voice) = tongue.voice() {
        argv.push("--voice".into());
        argv.push(voice.into());
    }
    argv.push("--speaker-identity".into());
    argv.push("synthetic".into());
    argv.push("--instruct".into());
    argv.push(tongue.instruct().into());
}

pub fn sidecar_argv_for(bin: &Path, talker: &Path, codec: &Path) -> Vec<String> {
    sidecar_argv_tongue(Tongue::Vivian, bin, talker, codec)
}

pub fn sidecar_argv_tongue(
    tongue: Tongue,
    bin: &Path,
    talker: &Path,
    codec: &Path,
) -> Vec<String> {
    let mut argv = Vec::new();
    push_common(&mut argv, bin, tongue, talker, codec, max_threads());
    argv.insert(1, "--server".into());
    argv.extend([
        "--host".into(),
        "127.0.0.1".into(),
        "--port".into(),
        PORT.to_string(),
        "--tts-max-input-chars".into(),
        CLIP_CAP.to_string(),
    ]);
    argv
}

pub fn sidecar_argv() -> Vec<String> {
    sidecar_argv_tongue(Tongue::Vivian, &bin(), &talker_for(Tongue::Vivian), &codec())
}

pub fn oneshot_argv_for(
    bin: &Path,
    talker: &Path,
    codec: &Path,
    text: &str,
    wav: &Path,
) -> Vec<String> {
    oneshot_argv_tongue(Tongue::Vivian, bin, talker, codec, text, wav)
}

pub fn oneshot_argv_tongue(
    tongue: Tongue,
    bin: &Path,
    talker: &Path,
    codec: &Path,
    text: &str,
    wav: &Path,
) -> Vec<String> {
    let mut argv = Vec::new();
    let n = thread_count(text.chars().count());
    push_common(&mut argv, bin, tongue, talker, codec, n);
    argv.extend([
        "--tts".into(),
        text.to_string(),
        "--tts-output".into(),
        wav.display().to_string(),
    ]);
    argv
}

pub fn oneshot_argv(text: &str, wav: &Path) -> Vec<String> {
    oneshot_argv_tongue(
        Tongue::Vivian,
        &bin(),
        &talker_for(Tongue::Vivian),
        &codec(),
        text,
        wav,
    )
}

fn tongue_addr() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], PORT))
}

pub fn sidecar_up() -> bool {
    TcpStream::connect_timeout(&tongue_addr(), Duration::from_millis(250)).is_ok()
}

fn apply_guest_env(cmd: &mut Command, threads: u16) {
    cmd.env("PATH", paths::house_path());
    cmd.env("LD_LIBRARY_PATH", lib_dir());
    cmd.env("OMP_NUM_THREADS", threads.to_string());
    cmd.env("OPENBLAS_NUM_THREADS", threads.to_string());
    cmd.current_dir(lib_dir());
    cmd.stdin(Stdio::null());
}

fn reap_if_dead(state: &mut Sidecar) {
    if let Some(c) = state.child.as_mut() {
        match c.try_wait() {
            Ok(Some(_)) | Err(_) => {
                state.child = None;
                state.tongue = None;
            }
            Ok(None) => {}
        }
    }
}

fn kill_child(child: &mut Child) {
    #[cfg(unix)]
    {
        let pid = child.id() as i32;
        extern "C" {
            fn killpg(pgrp: i32, sig: i32) -> i32;
        }
        const SIGTERM: i32 = 15;
        const SIGKILL: i32 = 9;
        unsafe {
            let _ = killpg(pid, SIGTERM);
        }
        let _ = child.wait();
        unsafe {
            let _ = killpg(pid, SIGKILL);
        }
    }
    #[cfg(not(unix))]
    {
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn steal_port() {
    let _ = Command::new("pkill")
        .args([
            "-f",
            "crispasr --server --no-gpu --host 127.0.0.1 --port 8084",
        ])
        .status();
    for _ in 0..25 {
        if !sidecar_up() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

pub fn stop_sidecar() {
    if cfg!(test) {
        return;
    }
    let mut g = sidecar_cell().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(mut child) = g.child.take() {
        kill_child(&mut child);
    }
    g.tongue = None;
    drop(g);
    if sidecar_up() {
        steal_port();
    }
}

fn spawn_sidecar(tongue: Tongue) -> Result<Child, String> {
    if !ready_tongue(tongue) {
        return Err(sleeping_reason_tongue(tongue));
    }
    let dir = paths::cache_dir().join("speak");
    fs::create_dir_all(&dir).map_err(|e| format!("pixie: tongue cache: {e}"))?;
    let log_path = sidecar_log();
    let log = fs::File::create(&log_path).map_err(|e| format!("pixie: tongue log: {e}"))?;
    let argv = sidecar_argv_tongue(tongue, &bin(), &talker_for(tongue), &codec());
    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..]);
    apply_guest_env(&mut cmd, max_threads());
    cmd.stdout(Stdio::from(log.try_clone().map_err(|e| e.to_string())?));
    cmd.stderr(Stdio::from(log));
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    cmd.spawn().map_err(|e| format!("pixie: tongue guest: {e}"))
}

fn wait_sidecar(child: &mut Child, timeout: Duration) -> Result<(), String> {
    let t0 = Instant::now();
    loop {
        if sidecar_up() {
            return Ok(());
        }
        match child.try_wait() {
            Ok(Some(st)) => {
                return Err(format!(
                    "pixie: tongue guest exited ({st}) · next: {}",
                    sidecar_log().display()
                ));
            }
            Err(e) => return Err(format!("pixie: tongue guest: {e}")),
            Ok(None) => {}
        }
        if t0.elapsed() >= timeout {
            return Err(
                "pixie: tongue guest stayed quiet · next: check ~/.cache/pixie/speak/sidecar.log"
                    .into(),
            );
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

pub fn ensure_sidecar() -> Result<(), String> {
    ensure_tongue(Tongue::Vivian)
}

pub fn ensure_tongue(tongue: Tongue) -> Result<(), String> {
    if cfg!(test) {
        return Ok(());
    }
    let mut g = sidecar_cell().lock().unwrap_or_else(|e| e.into_inner());
    reap_if_dead(&mut g);
    if g.child.is_some() && g.tongue == Some(tongue) && sidecar_up() {
        return Ok(());
    }
    if let Some(mut child) = g.child.take() {
        kill_child(&mut child);
    }
    g.tongue = None;
    drop(g);
    if sidecar_up() {
        steal_port();
    }
    let wait = match tongue {
        Tongue::Vivian => Duration::from_secs(90),
        Tongue::Design => Duration::from_secs(120),
    };
    let mut child = spawn_sidecar(tongue)?;
    if let Err(e) = wait_sidecar(&mut child, wait) {
        kill_child(&mut child);
        return Err(e);
    }
    let mut g = sidecar_cell().lock().unwrap_or_else(|e| e.into_inner());
    g.child = Some(child);
    g.tongue = Some(tongue);
    Ok(())
}

fn wav_ok(path: &Path) -> bool {
    matches!(fs::read(path), Ok(b) if b.len() > 44 && b.starts_with(b"RIFF"))
}

fn synth_http(tongue: Tongue, clip: &str, wav: &Path) -> Result<(), String> {
    let mut body = json!({
        "model": tongue.backend(),
        "input": clip,
        "instructions": tongue.instruct(),
        "instruct": tongue.instruct(),
        "response_format": "wav",
    });
    if let Some(voice) = tongue.voice() {
        body["voice"] = json!(voice);
    }
    let body = body.to_string();
    let url = format!("http://127.0.0.1:{PORT}/v1/audio/speech");
    let out = Command::new("curl")
        .args([
            "-fsS",
            "--max-time",
            "180",
            "-H",
            "Content-Type: application/json",
            "--data-binary",
            "@-",
            "--output",
            &wav.display().to_string(),
            &url,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(body.as_bytes())?;
            }
            child.wait_with_output()
        })
        .map_err(|e| format!("pixie: tongue http: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let err = err.trim();
        if err.is_empty() {
            return Err("pixie: tongue http failed".into());
        }
        return Err(format!(
            "pixie: tongue http: {}",
            err.chars().take(160).collect::<String>()
        ));
    }
    if !wav_ok(wav) {
        return Err("pixie: tongue http wrote no wav".into());
    }
    Ok(())
}

fn synth_oneshot(tongue: Tongue, clip: &str, wav: &Path) -> Result<(), String> {
    let argv = oneshot_argv_tongue(
        tongue,
        &bin(),
        &talker_for(tongue),
        &codec(),
        clip,
        wav,
    );
    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..]);
    apply_guest_env(&mut cmd, thread_count(clip.chars().count()));
    cmd.stdout(Stdio::null());
    cmd.stderr(Stdio::piped());
    let out = cmd
        .output()
        .map_err(|e| format!("pixie: tongue oneshot: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let err = err.trim();
        if err.is_empty() {
            return Err("pixie: tongue oneshot failed".into());
        }
        return Err(format!(
            "pixie: tongue oneshot: {}",
            err.chars().take(160).collect::<String>()
        ));
    }
    if !wav_ok(wav) {
        return Err("pixie: tongue oneshot wrote no wav".into());
    }
    Ok(())
}

fn play_wav(wav: &Path) -> Result<(), String> {
    let path = wav.display().to_string();
    for bin in ["paplay", "pw-play"] {
        let st = Command::new(bin)
            .arg(&path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        match st {
            Ok(s) if s.success() => return Ok(()),
            _ => continue,
        }
    }
    Err("pixie: tongue play failed · next: paplay or pw-play".into())
}

/// Queue spoken sentences (overlap play). Tests never start CrispASR or play audio.
pub fn say(text: &str) -> Result<(), String> {
    say_tongue(Tongue::Design, text)
}

pub fn say_tongue(tongue: Tongue, text: &str) -> Result<(), String> {
    if cfg!(test) {
        return Ok(());
    }
    let clip = spoken_clip(text);
    if clip.is_empty() {
        return Ok(());
    }
    if !ready_tongue(tongue) {
        return Err(sleeping_reason_tongue(tongue));
    }
    let (sents, rest) = take_speech_units(&clip, true);
    for s in sents {
        enqueue(tongue, s);
    }
    if !rest.trim().is_empty() {
        enqueue(tongue, rest.trim().to_string());
    }
    let gen = streamer().lock().map(|g| g.gen).unwrap_or(0);
    if let Some(tx) = jobs() {
        let _ = tx.send(Job::Flush { gen });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spoken_clip_keeps_a_sentence() {
        assert_eq!(
            spoken_clip("The hearth is warm tonight."),
            "The hearth is warm tonight."
        );
    }

    #[test]
    fn spoken_clip_drops_tool_json() {
        assert!(spoken_clip(r#"{"name":"execute_bash","arguments":{}}"#).is_empty());
        assert!(spoken_clip("<tool_call>look</tool_call>").is_empty());
        assert!(spoken_clip("```json\n{\"name\":\"look\"}\n```").is_empty());
    }

    #[test]
    fn spoken_clip_drops_red_facts() {
        assert!(spoken_clip("the wifi password is secret").is_empty());
        assert!(spoken_clip("NIF 123").is_empty());
    }

    #[test]
    fn spoken_clip_caps_length() {
        let long = "word ".repeat(400);
        let clip = spoken_clip(&long);
        assert!(!clip.is_empty());
        assert!(clip.chars().count() <= CLIP_CAP);
    }

    fn assert_cpu_guest(argv: &[String]) {
        let joined = argv.join(" ");
        assert!(joined.contains("--no-gpu"), "{joined}");
        assert!(
            argv.windows(2).any(|w| w[0] == "-t"
                && w[1].parse::<u16>().ok().is_some_and(|n| n >= 2 && n <= max_threads())),
            "{joined}"
        );
        assert!(joined.contains("--codec-model"), "{joined}");
        assert!(!joined.contains("danny"), "{joined}");
        assert!(!joined.to_lowercase().contains("menagerie"), "{joined}");
        assert!(!joined.contains("--tts-play"), "{joined}");
        assert!(!joined.contains("-m auto"), "{joined}");
        assert!(!joined.contains("siren"), "{joined}");
    }

    #[test]
    fn sidecar_stays_cpu_customvoice() {
        let argv = sidecar_argv_tongue(
            Tongue::Vivian,
            Path::new("/tmp/crispasr"),
            Path::new("/tmp/talker.gguf"),
            Path::new("/tmp/codec.gguf"),
        );
        let joined = argv.join(" ");
        assert_cpu_guest(&argv);
        assert!(joined.contains("qwen3-tts-customvoice"), "{joined}");
        assert!(joined.contains("8084"), "{joined}");
        assert!(joined.contains("vivian"), "{joined}");
        assert!(joined.contains("--voice"), "{joined}");
        assert!(joined.contains("--speaker-identity"), "{joined}");
        assert!(joined.contains("synthetic"), "{joined}");
        assert!(
            argv.windows(2)
                .any(|w| w[0] == "-t" && w[1] == max_threads().to_string()),
            "sidecar stays at cores-2 ({joined})"
        );
    }

    #[test]
    fn sidecar_design_drops_vivian_voice_flag() {
        let argv = sidecar_argv_tongue(
            Tongue::Design,
            Path::new("/tmp/crispasr"),
            Path::new("/tmp/design.gguf"),
            Path::new("/tmp/codec.gguf"),
        );
        let joined = argv.join(" ");
        assert_cpu_guest(&argv);
        assert!(joined.contains("qwen3-tts-1.7b-voicedesign"), "{joined}");
        assert!(!joined.contains("vivian"), "{joined}");
        assert!(!joined.contains("--voice"), "{joined}");
        assert!(joined.contains("--instruct"), "{joined}");
    }

    #[test]
    fn oneshot_stays_cpu_customvoice() {
        let argv = oneshot_argv_for(
            Path::new("/tmp/crispasr"),
            Path::new("/tmp/talker.gguf"),
            Path::new("/tmp/codec.gguf"),
            "hello",
            Path::new("/tmp/last.wav"),
        );
        let joined = argv.join(" ");
        assert!(joined.contains("--no-gpu"), "{joined}");
        assert!(joined.contains("-t"), "{joined}");
        assert!(joined.contains("qwen3-tts-customvoice"), "{joined}");
        assert!(joined.contains("--codec-model"), "{joined}");
        assert!(joined.contains("--tts"), "{joined}");
        assert!(!joined.contains("danny"), "{joined}");
        assert!(!joined.contains("menagerie"), "{joined}");
    }

    #[test]
    fn missing_guest_sleeps() {
        let nope = Path::new("/no/such/crispasr-guest");
        assert!(!ready_from(nope, nope, nope));
        let why = sleeping_reason_from(nope, nope, nope);
        assert!(why.contains("tongue is still sleeping"), "{why}");
        assert!(why.contains("next:"), "{why}");
    }

    #[test]
    fn test_ensure_and_say_are_nops() {
        assert!(ensure_sidecar().is_ok());
        assert!(ensure_tongue(Tongue::Design).is_ok());
        assert!(say("hello from a test").is_ok());
        assert!(say_tongue(Tongue::Design, "hello design").is_ok());
        stop_sidecar();
    }

    #[test]
    fn tongue_from_str_defaults_design() {
        assert_eq!(Tongue::from_str("vivian"), Tongue::Vivian);
        assert_eq!(Tongue::from_str("design"), Tongue::Design);
        assert_eq!(Tongue::from_str(""), Tongue::Design);
        assert_eq!(Tongue::from_str("pixie"), Tongue::Design);
    }

    #[test]
    fn take_sentences_keeps_a_short_tail() {
        let (s, rest) = take_sentences("The hearth is warm. I kept the kettle");
        assert_eq!(s, vec!["The hearth is warm.".to_string()]);
        assert!(rest.contains("kettle"), "{rest}");
        let (none, tail) = take_sentences("Hi.");
        assert!(none.is_empty(), "{none:?}");
        assert_eq!(tail.trim(), "Hi.");
        let (all, empty) = take_sentences("Quick and kind. A little sparkle.");
        assert_eq!(all.len(), 2, "{all:?}");
        assert!(empty.trim().is_empty(), "{empty}");
    }

    #[test]
    fn first_clause_goes_immediately() {
        let (got, rest) = take_speech_units("Hi.", true);
        assert_eq!(got, vec!["Hi.".to_string()]);
        assert!(rest.trim().is_empty(), "{rest}");
        let (got, rest) = take_speech_units(
            "Once upon a warm hearth, the kettle sang for everyone nearby.",
            true,
        );
        assert!(!got.is_empty(), "{got:?}");
        assert!(got[0].contains("hearth"), "{got:?}");
        assert!(
            rest.contains("kettle") || got.iter().any(|s| s.contains("kettle")),
            "{got:?} {rest}"
        );
    }

    #[test]
    fn spoken_plain_maps_laughs_and_drops_sparkles() {
        let s = spoken_plain("Hello 😂✨ world 🤣");
        assert!(s.contains("ha ha"), "{s}");
        assert!(s.contains("Hello"), "{s}");
        assert!(s.contains("world"), "{s}");
        assert!(!s.contains('✨'), "{s}");
        assert!(!s.contains('😂'), "{s}");
    }

    #[test]
    fn spoken_plain_maps_more_than_laughs() {
        let s = spoken_plain("Wait 🤔 I yawn 😴 achoo 🤧 shock 😮 party 🎉 sad 😢 shh 🤫");
        assert!(s.contains("hmm"), "{s}");
        assert!(s.contains("yawn"), "{s}");
        assert!(s.contains("achoo"), "{s}");
        assert!(s.contains("oh"), "{s}");
        assert!(s.contains("yay"), "{s}");
        assert!(s.contains("aw"), "{s}");
        assert!(s.contains("shh"), "{s}");
        assert!(!s.contains('🤔'), "{s}");
    }

    #[test]
    fn spoken_plain_strips_technical_junk() {
        let s = spoken_plain(
            "See [docs](https://example.com/x) and `src/speak.rs` plus ~/.cache/pixie/speak/last.wav **bold**.",
        );
        assert!(s.contains("See"), "{s}");
        assert!(s.contains("docs"), "{s}");
        assert!(!s.contains("example.com"), "{s}");
        assert!(!s.contains("last.wav"), "{s}");
        assert!(!s.contains("**"), "{s}");
        assert!(!s.contains('`'), "{s}");
    }

    #[test]
    fn short_clips_use_fewer_threads() {
        assert!(max_threads() >= 2);
        assert!(thread_count(20) <= thread_count(500));
        assert!(thread_count(500) <= max_threads());
        let short = oneshot_argv_tongue(
            Tongue::Vivian,
            Path::new("/tmp/crispasr"),
            Path::new("/tmp/talker.gguf"),
            Path::new("/tmp/codec.gguf"),
            "hi",
            Path::new("/tmp/last.wav"),
        );
        let long = oneshot_argv_tongue(
            Tongue::Vivian,
            Path::new("/tmp/crispasr"),
            Path::new("/tmp/talker.gguf"),
            Path::new("/tmp/codec.gguf"),
            &"word ".repeat(80),
            Path::new("/tmp/last.wav"),
        );
        let t_short = short
            .windows(2)
            .find(|w| w[0] == "-t")
            .and_then(|w| w[1].parse::<u16>().ok())
            .unwrap();
        let t_long = long
            .windows(2)
            .find(|w| w[0] == "-t")
            .and_then(|w| w[1].parse::<u16>().ok())
            .unwrap();
        assert!(t_short <= t_long, "{t_short} vs {t_long}");
        assert_eq!(t_long, max_threads());
    }
}
