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
pub const SPEAKER: &str = "vivian";
pub const BACKEND: &str = "qwen3-tts-customvoice";
pub const INSTRUCT: &str = "Bright small all-ages fairy, quick and kind, creature not a child.";
pub const CLIP_CAP: usize = 800;

const TALKER_NAME: &str = "qwen3-tts-12hz-0.6b-customvoice-q8_0.gguf";
const CODEC_NAME: &str = "qwen3-tts-tokenizer-12hz.gguf";

fn sidecar_cell() -> &'static Mutex<Option<Child>> {
    static C: OnceLock<Mutex<Option<Child>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(None))
}

pub fn bin() -> PathBuf {
    paths::home().join(".local/lib/crispasr/crispasr")
}

pub fn lib_dir() -> PathBuf {
    paths::home().join(".local/lib/crispasr")
}

pub fn talker() -> PathBuf {
    paths::data_dir().join("models").join(TALKER_NAME)
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

pub fn ready() -> bool {
    ready_from(&bin(), &talker(), &codec())
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
    sleeping_reason_from(&bin(), &talker(), &codec())
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

pub fn sidecar_argv_for(bin: &Path, talker: &Path, codec: &Path) -> Vec<String> {
    vec![
        bin.display().to_string(),
        "--server".into(),
        "--no-gpu".into(),
        "--host".into(),
        "127.0.0.1".into(),
        "--port".into(),
        PORT.to_string(),
        "--backend".into(),
        BACKEND.into(),
        "-m".into(),
        talker.display().to_string(),
        "--codec-model".into(),
        codec.display().to_string(),
        "--voice".into(),
        SPEAKER.into(),
        "--speaker-identity".into(),
        "synthetic".into(),
        "--instruct".into(),
        INSTRUCT.into(),
        "--tts-max-input-chars".into(),
        CLIP_CAP.to_string(),
    ]
}

pub fn sidecar_argv() -> Vec<String> {
    sidecar_argv_for(&bin(), &talker(), &codec())
}

pub fn oneshot_argv_for(bin: &Path, talker: &Path, codec: &Path, text: &str, wav: &Path) -> Vec<String> {
    vec![
        bin.display().to_string(),
        "--no-gpu".into(),
        "--backend".into(),
        BACKEND.into(),
        "-m".into(),
        talker.display().to_string(),
        "--codec-model".into(),
        codec.display().to_string(),
        "--voice".into(),
        SPEAKER.into(),
        "--speaker-identity".into(),
        "synthetic".into(),
        "--instruct".into(),
        INSTRUCT.into(),
        "--tts".into(),
        text.to_string(),
        "--tts-output".into(),
        wav.display().to_string(),
    ]
}

pub fn oneshot_argv(text: &str, wav: &Path) -> Vec<String> {
    oneshot_argv_for(&bin(), &talker(), &codec(), text, wav)
}

fn tongue_addr() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], PORT))
}

pub fn sidecar_up() -> bool {
    TcpStream::connect_timeout(&tongue_addr(), Duration::from_millis(250)).is_ok()
}

fn apply_guest_env(cmd: &mut Command) {
    cmd.env("PATH", paths::house_path());
    cmd.env("LD_LIBRARY_PATH", lib_dir());
    cmd.current_dir(lib_dir());
    cmd.stdin(Stdio::null());
}

fn reap_if_dead(child: &mut Option<Child>) {
    if let Some(c) = child.as_mut() {
        match c.try_wait() {
            Ok(Some(_)) | Err(_) => {
                *child = None;
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

pub fn stop_sidecar() {
    if cfg!(test) {
        return;
    }
    let mut g = sidecar_cell()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if let Some(mut child) = g.take() {
        kill_child(&mut child);
    }
}

fn spawn_sidecar() -> Result<Child, String> {
    if !ready() {
        return Err(sleeping_reason());
    }
    let dir = paths::cache_dir().join("speak");
    fs::create_dir_all(&dir).map_err(|e| format!("pixie: tongue cache: {e}"))?;
    let log_path = sidecar_log();
    let log = fs::File::create(&log_path).map_err(|e| format!("pixie: tongue log: {e}"))?;
    let argv = sidecar_argv();
    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..]);
    apply_guest_env(&mut cmd);
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
    if cfg!(test) {
        return Ok(());
    }
    if sidecar_up() {
        return Ok(());
    }
    let mut g = sidecar_cell()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    reap_if_dead(&mut g);
    if g.is_some() && sidecar_up() {
        return Ok(());
    }
    if let Some(mut dead) = g.take() {
        kill_child(&mut dead);
    }
    let mut child = spawn_sidecar()?;
    if let Err(e) = wait_sidecar(&mut child, Duration::from_secs(90)) {
        kill_child(&mut child);
        return Err(e);
    }
    *g = Some(child);
    Ok(())
}

fn wav_ok(path: &Path) -> bool {
    match fs::read(path) {
        Ok(b) if b.len() > 44 && b.starts_with(b"RIFF") => true,
        _ => false,
    }
}

fn synth_http(clip: &str, wav: &Path) -> Result<(), String> {
    let body = json!({
        "model": BACKEND,
        "input": clip,
        "voice": SPEAKER,
        "instructions": INSTRUCT,
        "instruct": INSTRUCT,
        "response_format": "wav",
    })
    .to_string();
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
        return Err(format!("pixie: tongue http: {}", err.chars().take(160).collect::<String>()));
    }
    if !wav_ok(wav) {
        return Err("pixie: tongue http wrote no wav".into());
    }
    Ok(())
}

fn synth_oneshot(clip: &str, wav: &Path) -> Result<(), String> {
    let argv = oneshot_argv(clip, wav);
    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..]);
    apply_guest_env(&mut cmd);
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
    if cfg!(test) {
        return Ok(());
    }
    let clip = spoken_clip(text);
    if clip.is_empty() {
        return Ok(());
    }
    if !ready() {
        return Err(sleeping_reason());
    }
    let dir = paths::cache_dir().join("speak");
    fs::create_dir_all(&dir).map_err(|e| format!("pixie: tongue cache: {e}"))?;
    let wav = cache_wav();
    let _ = fs::remove_file(&wav);
    match ensure_sidecar() {
        Ok(()) => {
            if synth_http(&clip, &wav).is_err() {
                synth_oneshot(&clip, &wav)?;
            }
        }
        Err(_) => synth_oneshot(&clip, &wav)?,
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

    #[test]
    fn sidecar_stays_cpu_customvoice() {
        let argv = sidecar_argv_for(
            Path::new("/tmp/crispasr"),
            Path::new("/tmp/talker.gguf"),
            Path::new("/tmp/codec.gguf"),
        );
        let joined = argv.join(" ");
        assert!(joined.contains("--no-gpu"), "{joined}");
        assert!(joined.contains("--backend"), "{joined}");
        assert!(joined.contains("qwen3-tts-customvoice"), "{joined}");
        assert!(joined.contains("--codec-model"), "{joined}");
        assert!(joined.contains("8084"), "{joined}");
        assert!(joined.contains("vivian"), "{joined}");
        assert!(joined.contains("--speaker-identity"), "{joined}");
        assert!(joined.contains("synthetic"), "{joined}");
        assert!(!joined.contains("danny"), "{joined}");
        assert!(!joined.to_lowercase().contains("menagerie"), "{joined}");
        assert!(!joined.contains("--tts-play"), "{joined}");
        assert!(!joined.contains("-m auto") && !joined.contains(" auto"), "{joined}");
        assert!(!joined.contains("siren"), "{joined}");
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
        assert!(say("hello from a test").is_ok());
        stop_sidecar();
    }
}
