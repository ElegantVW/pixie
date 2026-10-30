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
            "design" | "voicedesign" | "pixie" => Tongue::Design,
            _ => Tongue::Vivian,
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

/// Synth + paplay. Tests never start CrispASR or play audio.
pub fn say(text: &str) -> Result<(), String> {
    say_tongue(Tongue::Vivian, text)
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
    let dir = paths::cache_dir().join("speak");
    fs::create_dir_all(&dir).map_err(|e| format!("pixie: tongue cache: {e}"))?;
    let wav = cache_wav();
    let _ = fs::remove_file(&wav);
    match ensure_tongue(tongue) {
        Ok(()) => {
            if synth_http(tongue, &clip, &wav).is_err() {
                synth_oneshot(tongue, &clip, &wav)?;
            }
        }
        Err(_) => synth_oneshot(tongue, &clip, &wav)?,
    }
    play_wav(&wav)
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
    fn tongue_from_str_defaults_vivian() {
        assert_eq!(Tongue::from_str("vivian"), Tongue::Vivian);
        assert_eq!(Tongue::from_str("design"), Tongue::Design);
        assert_eq!(Tongue::from_str(""), Tongue::Vivian);
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
