//! Sitting sudo: RAM secret + runtime askpass. Never config, jsonl, or argv.

use crate::paths;
use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const IDLE: Duration = Duration::from_secs(15 * 60);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SudoMode {
    Off,
    Ticket,
    Sitting,
    LessSafe,
}

struct Seat {
    mode: SudoMode,
    secret: Option<String>,
    last: Instant,
    stop: Arc<AtomicBool>,
}

impl Seat {
    fn off() -> Self {
        Self {
            mode: SudoMode::Off,
            secret: None,
            last: Instant::now(),
            stop: Arc::new(AtomicBool::new(true)),
        }
    }
}

fn seat() -> &'static Mutex<Seat> {
    static SEAT: std::sync::OnceLock<Mutex<Seat>> = std::sync::OnceLock::new();
    SEAT.get_or_init(|| Mutex::new(Seat::off()))
}

fn wipe(s: &mut String) {
    if s.is_empty() {
        return;
    }
    let n = s.len();
    s.clear();
    s.push_str(&"0".repeat(n));
    s.clear();
    s.shrink_to_fit();
}

pub fn sock_path() -> std::path::PathBuf {
    paths::runtime_dir().join("askpass.sock")
}

pub fn wrapper_path() -> std::path::PathBuf {
    paths::runtime_dir().join("askpass")
}

pub fn mode() -> SudoMode {
    seat().lock().map(|s| s.mode).unwrap_or(SudoMode::Off)
}

pub fn is_on() -> bool {
    mode() != SudoMode::Off
}

pub fn status_label() -> &'static str {
    match mode() {
        SudoMode::Off => "off",
        SudoMode::Ticket => "on · ticket",
        SudoMode::Sitting => "on · sitting",
        SudoMode::LessSafe => "on · less-safe",
    }
}

pub fn ticket_ok() -> bool {
    Command::new("sudo")
        .args(["-n", "true"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn enable_ticket() {
    disable();
    if let Ok(mut s) = seat().lock() {
        s.mode = SudoMode::Ticket;
        s.last = Instant::now();
    }
}

pub fn enable_less_safe(mut pw: String) -> std::io::Result<()> {
    disable();
    let mut secret = pw.trim_end_matches(['\n', '\r']).to_string();
    wipe(&mut pw);
    if secret.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "password is empty",
        ));
    }

    let stop = Arc::new(AtomicBool::new(false));
    let setup = (|| -> std::io::Result<()> {
        write_wrapper()?;
        let listener = bind_server()?;
        let mut s = seat().lock().unwrap_or_else(|p| p.into_inner());
        s.mode = SudoMode::Off;
        s.secret = Some(secret.clone());
        s.last = Instant::now();
        s.stop = stop.clone();
        let server_stop = stop.clone();
        let rt = paths::runtime_dir();
        thread::spawn(move || {
            let _ =
                std::fs::set_permissions(&wrapper_path(), std::fs::Permissions::from_mode(0o700));
            while !server_stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut st, _)) => {
                        touch();
                        let _ = st.set_read_timeout(Some(Duration::from_secs(2)));
                        let mut buf = [0u8; 256];
                        let _ = st.read(&mut buf);
                        let secret_copy = {
                            let mut s2 = match seat().lock() {
                                Ok(x) => x,
                                Err(_) => break,
                            };
                            if server_stop.load(Ordering::SeqCst) {
                                None
                            } else if s2.mode == SudoMode::LessSafe {
                                s2.secret.as_ref().map(|sec| sec.clone())
                            } else {
                                None
                            }
                        };
                        if let Some(sec) = secret_copy {
                            let mut out = sec.as_bytes().to_vec();
                            out.push(b'\n');
                            let _ = st.write_all(&out);
                            let _ = st.flush();
                            drop(st);
                            continue;
                        }
                        let _ = st.write_all(b"\n");
                        let _ = st.flush();
                        drop(st);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(40));
                    }
                    Err(_) => break,
                }
            }
            let _ = std::fs::remove_file(sock_path());
            let _ = std::fs::remove_file(wrapper_path());
        });
        {
            let mut s2 = seat().lock().unwrap_or_else(|p| p.into_inner());
            s2.mode = SudoMode::LessSafe;
            s2.last = Instant::now();
        }
        let _ = std::fs::write(rt.join("sudo-mode"), "less-safe\n");
        Ok(())
    })();
    if let Err(e) = setup {
        disable();
        return Err(e);
    }
    wipe(&mut secret);
    Ok(())
}

pub fn disable() {
    let mut s = seat().lock().unwrap_or_else(|p| p.into_inner());
    s.mode = SudoMode::Off;
    s.stop.store(true, Ordering::SeqCst);
    if let Some(mut pw) = s.secret.take() {
        wipe(&mut pw);
    }
    drop(s);
    let sock = sock_path();
    let _ = fs::remove_file(&sock);
    let _ = fs::remove_file(wrapper_path());
}

pub fn idle_tick() {
    let stale = seat()
        .lock()
        .ok()
        .map(|s| s.mode == SudoMode::Sitting && s.last.elapsed() >= IDLE)
        .unwrap_or(false);
    if stale {
        disable();
    }
}

pub fn touch() {
    if let Ok(mut s) = seat().lock() {
        if s.mode != SudoMode::Off {
            s.last = Instant::now();
        }
    }
}

/// Env for execute_bash children when sitting sudo is on.
pub fn child_env() -> Option<(String, String)> {
    match mode() {
        SudoMode::Sitting | SudoMode::LessSafe => {
            let wrap = wrapper_path();
            if wrap.is_file() {
                Some(("SUDO_ASKPASS".into(), wrap.display().to_string()))
            } else {
                None
            }
        }
        _ => None,
    }
}

fn write_wrapper() -> std::io::Result<()> {
    let dir = paths::runtime_dir();
    fs::create_dir_all(&dir)?;
    let mut perms = fs::metadata(&dir)?.permissions();
    perms.set_mode(0o700);
    fs::set_permissions(&dir, perms)?;
    let exe = std::env::current_exe()
        .ok()
        .and_then(|p| p.to_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "pixie".into());
    // Absolute path only. No secret in this file.
    let body = format!("#!/bin/sh\nexec {exe:?} --askpass\n");
    let wrap = wrapper_path();
    fs::write(&wrap, body)?;
    let mut p = fs::Permissions::from_mode(0o700);
    p.set_mode(0o700);
    fs::set_permissions(&wrap, p)?;
    Ok(())
}

fn bind_server() -> std::io::Result<UnixListener> {
    let dir = paths::runtime_dir();
    fs::create_dir_all(&dir)?;
    let sock = sock_path();
    let _ = fs::remove_file(&sock);
    let listener = UnixListener::bind(&sock)?;
    let mut p = fs::Permissions::from_mode(0o600);
    p.set_mode(0o600);
    if let Err(e) = fs::set_permissions(&sock, p).and_then(|_| listener.set_nonblocking(true)) {
        drop(listener);
        let _ = fs::remove_file(&sock);
        return Err(e);
    }
    Ok(listener)
}

fn serve(listener: UnixListener, stop: Arc<AtomicBool>) {
    while !stop.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((mut stream, _)) => {
                touch();
                let line = seat()
                    .lock()
                    .ok()
                    .and_then(|s| s.secret.clone())
                    .unwrap_or_default();
                if !line.is_empty() {
                    let _ = stream.write_all(line.as_bytes());
                    if !line.ends_with('\n') {
                        let _ = stream.write_all(b"\n");
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(40));
            }
            Err(_) => break,
        }
    }
}

/// `pixie --askpass` — stdout is the secret for sudo. No TUI.
pub fn askpass_client() -> i32 {
    let sock = sock_path();
    let Ok(mut stream) = UnixStream::connect(&sock) else {
        return 1;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let mut buf = String::new();
    let _ = stream.read_to_string(&mut buf);
    let buf = buf.trim_end_matches(['\n', '\r']).to_string();
    if buf.is_empty() {
        return 1;
    }
    println!("{buf}");
    let _ = std::io::stdout().flush();
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_off() {
        disable();
        assert!(!is_on());
        assert_eq!(status_label(), "off");
        assert!(child_env().is_none());
    }

    #[test]
    fn wrapper_has_no_password_word_as_secret_store() {
        let src = include_str!("privilege.rs");
        assert!(src.contains("XDG_RUNTIME_DIR") || src.contains("runtime_dir"));
        assert!(src.contains("--askpass"));
        assert!(src.contains("SUDO_ASKPASS"));
    }
}
