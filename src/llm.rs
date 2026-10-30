//! Loopback llama-server client. Abort = drop the socket.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub fn port() -> Option<u16> {
    match std::env::var("PIXIE_LLM_PORT") {
        Ok(port) => port.parse().ok(),
        Err(std::env::VarError::NotPresent) => Some(8080),
        Err(std::env::VarError::NotUnicode(_)) => None,
    }
}

pub fn host() -> String {
    std::env::var("PIXIE_LLM_HOST").unwrap_or_else(|_| "127.0.0.1".into())
}

fn endpoint(host: &str, port: Option<u16>) -> Option<std::net::SocketAddr> {
    let port = port?;
    let ip = host
        .parse()
        .or_else(|_| host.trim_start_matches('[').trim_end_matches(']').parse())
        .ok()?;
    Some(std::net::SocketAddr::new(ip, port))
}

fn configured_endpoint() -> Option<std::net::SocketAddr> {
    endpoint(&host(), port())
}

pub struct Abort {
    pub flag: Arc<AtomicBool>,
    stream: Arc<Mutex<Option<TcpStream>>>,
}

impl Abort {
    pub fn new() -> Self {
        Self {
            flag: Arc::new(AtomicBool::new(false)),
            stream: Arc::new(Mutex::new(None)),
        }
    }

    pub fn cancel(&self) {
        self.flag.store(true, Ordering::SeqCst);
        if let Ok(mut g) = self.stream.lock() {
            if let Some(s) = g.take() {
                let _ = s.shutdown(std::net::Shutdown::Both);
            }
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }
}

impl Clone for Abort {
    fn clone(&self) -> Self {
        Self {
            flag: self.flag.clone(),
            stream: self.stream.clone(),
        }
    }
}

pub fn llm_up() -> bool {
    let Some(endpoint) = configured_endpoint() else {
        return false;
    };
    let addr = endpoint.to_string();
    let Ok(mut s) = TcpStream::connect_timeout(&endpoint, Duration::from_secs(2)) else {
        return false;
    };
    let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
    let req = format!("GET /health HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    if s.write_all(req.as_bytes()).is_err() {
        return false;
    }

    let mut buf = String::new();
    let mut r = BufReader::new(s);
    if r.read_line(&mut buf).is_err() {
        return false;
    }

    health_line_ok(&buf)
}

pub fn health_line_ok(line: &str) -> bool {
    line.contains("200") && !line.contains("503")
}

pub fn ensure_llm_up() -> bool {
    if configured_endpoint().is_none() {
        return false;
    }
    if llm_up() {
        return true;
    }
    let menagerie = crate::paths::home().join("bin/menagerie");
    let _ = Command::new(if menagerie.is_file() {
        menagerie.as_os_str()
    } else {
        std::ffi::OsStr::new("menagerie")
    })
    .args(["ensure", "pixie"])
    .output();
    for _ in 0..60 {
        if llm_up() {
            return true;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    llm_up()
}

pub fn unload() {
    let menagerie = crate::paths::home().join("bin/menagerie");
    let _ = Command::new(if menagerie.is_file() {
        menagerie.as_os_str()
    } else {
        std::ffi::OsStr::new("menagerie")
    })
    .args(["stop", "pixie"])
    .output();
}

pub struct StreamOut {
    pub text: String,
    pub err: Option<String>,
}

pub fn stream_chat(
    messages: &[Value],
    tools: Option<Value>,
    abort: &Abort,
    mut on_chunk: impl FnMut(&str),
) -> StreamOut {
    let payload = chat_payload(messages, tools).to_string();
    let Some(sock_addr) = configured_endpoint() else {
        return StreamOut {
            text: String::new(),
            err: Some("invalid PIXIE_LLM_HOST or PIXIE_LLM_PORT".into()),
        };
    };
    let addr = sock_addr.to_string();
    let mut stream = match TcpStream::connect_timeout(&sock_addr, Duration::from_secs(5)) {
        Ok(s) => s,
        Err(e) => {
            return StreamOut {
                text: String::new(),
                err: Some(e.to_string()),
            };
        }
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(120)));
    let _ = stream.set_nodelay(true);
    let req = format!(
        "POST /v1/chat/completions HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
        payload.len()
    );
    if let Err(e) = stream.write_all(req.as_bytes()) {
        return StreamOut {
            text: String::new(),
            err: Some(e.to_string()),
        };
    }
    if let Ok(clone) = stream.try_clone() {
        if let Ok(mut g) = abort.stream.lock() {
            *g = Some(clone);
        }
    }
    let mut reader = BufReader::new(stream);
    let mut header = String::new();
    loop {
        header.clear();
        match reader.read_line(&mut header) {
            Ok(0) => break,
            Ok(_) => {
                if header == "\r\n" || header == "\n" {
                    break;
                }
            }
            Err(e) => {
                if abort.is_cancelled() {
                    return StreamOut {
                        text: String::new(),
                        err: None,
                    };
                }
                return StreamOut {
                    text: String::new(),
                    err: Some(e.to_string()),
                };
            }
        }
    }
    let mut full = String::new();
    let mut acc_tools: std::collections::BTreeMap<i64, (String, String)> =
        std::collections::BTreeMap::new();
    let mut line = String::new();
    loop {
        if abort.is_cancelled() {
            break;
        }
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Err(_) => break,
            Ok(_) => {}
        }
        let t = line.trim();
        if !t.starts_with("data:") {
            continue;
        }
        let payload = t[5..].trim();
        if payload.is_empty() || payload == "[DONE]" {
            continue;
        }
        let Ok(d) = serde_json::from_str::<Value>(payload) else {
            continue;
        };
        for ch in d
            .get("choices")
            .and_then(|x| x.as_array())
            .cloned()
            .unwrap_or_default()
        {
            let delta = ch.get("delta").cloned().unwrap_or(json!({}));
            if let Some(tcs) = delta.get("tool_calls").and_then(|x| x.as_array()) {
                for tc in tcs {
                    let idx = tc.get("index").and_then(|x| x.as_i64()).unwrap_or(0);
                    let slot = acc_tools
                        .entry(idx)
                        .or_insert((String::new(), String::new()));
                    if let Some(n) = tc.pointer("/function/name").and_then(|x| x.as_str()) {
                        slot.0.push_str(n);
                    }
                    if let Some(a) = tc.pointer("/function/arguments").and_then(|x| x.as_str()) {
                        slot.1.push_str(a);
                    }
                }
            }
            if let Some(c) = delta.get("content").and_then(|x| x.as_str()) {
                if !c.is_empty() {
                    full.push_str(c);
                    on_chunk(c);
                }
            }
        }
    }
    if let Ok(mut g) = abort.stream.lock() {
        *g = None;
    }
    if !acc_tools.is_empty() && crate::mind::extract_tool_calls(&full, 1).is_empty() {
        let pairs: Vec<(String, String)> = acc_tools.into_values().collect();
        let synth = synth_tool_calls(&pairs);
        if !synth.is_empty() {
            if full.is_empty() {
                on_chunk(&synth);
                full = synth;
            } else {
                full.push('\n');
                full.push_str(&synth);
            }
        }
    }
    StreamOut {
        text: full,
        err: None,
    }
}

/// House sampler body. `tools` is ignored on the wire — Pixie parses JSON
/// from the assistant text (`mind::extract_tool_calls`). llama.cpp native
/// tool_calls are a guest we are deleting.
pub fn chat_payload(messages: &[Value], _tools: Option<Value>) -> Value {
    json!({
        "messages": messages,
        "temperature": 0.2,
        "max_tokens": 2048,
        "stream": true,
    })
}

pub fn mouth_up(port: u16) -> bool {
    let addr = format!("127.0.0.1:{port}");
    let Ok(sock) = addr.parse() else {
        return false;
    };
    TcpStream::connect_timeout(&sock, Duration::from_millis(400)).is_ok()
}

fn wake_kur(port: u16) {
    let addr = format!("127.0.0.1:{port}");
    let Ok(sock) = addr.parse::<std::net::SocketAddr>() else {
        return;
    };
    if TcpStream::connect_timeout(&sock, Duration::from_millis(400)).is_ok() {
        return;
    }
    let menagerie = crate::paths::home().join("bin/menagerie");
    let _ = Command::new(if menagerie.is_file() {
        menagerie.as_os_str()
    } else {
        std::ffi::OsStr::new("menagerie")
    })
    .args(["ensure", "kur"])
    .output();
    let deadline = std::time::Instant::now() + Duration::from_secs(8);
    while std::time::Instant::now() < deadline {
        if TcpStream::connect_timeout(&sock, Duration::from_millis(200)).is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// Short non-stream completion on another mouth (kur/smolLM). None on miss.
pub fn complete_on(
    port: u16,
    messages: &[Value],
    max_tokens: u32,
    timeout_s: u64,
) -> Option<String> {
    wake_kur(port);
    complete_on_raw(port, messages, max_tokens, timeout_s, 0.85)
}

/// Same as complete_on but never wakes menagerie. Skip if the mouth is down.
pub fn complete_on_if_up(
    port: u16,
    messages: &[Value],
    max_tokens: u32,
    timeout_s: u64,
) -> Option<String> {
    if !mouth_up(port) {
        return None;
    }
    complete_on_raw(port, messages, max_tokens, timeout_s, 0.85)
}

/// Non-stream completion with an explicit temperature. Skip if the mouth is down.
pub fn complete_on_if_up_temp(
    port: u16,
    messages: &[Value],
    max_tokens: u32,
    timeout_s: u64,
    temperature: f64,
) -> Option<String> {
    if !mouth_up(port) {
        return None;
    }
    complete_on_raw(port, messages, max_tokens, timeout_s, temperature)
}

fn complete_on_raw(
    port: u16,
    messages: &[Value],
    max_tokens: u32,
    timeout_s: u64,
    temperature: f64,
) -> Option<String> {
    let addr = format!("127.0.0.1:{port}");
    let sock: std::net::SocketAddr = addr.parse().ok()?;
    let payload = json!({
        "messages": messages,
        "temperature": temperature,
        "max_tokens": max_tokens,
        "stream": false,
    })
    .to_string();
    let mut stream = TcpStream::connect_timeout(&sock, Duration::from_secs(timeout_s)).ok()?;
    let _ = stream.set_read_timeout(Some(Duration::from_secs(timeout_s)));
    let _ = stream.set_nodelay(true);
    let req = format!(
        "POST /v1/chat/completions HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
        payload.len()
    );
    stream.write_all(req.as_bytes()).ok()?;
    let mut reader = BufReader::new(stream);
    let mut header = String::new();
    loop {
        header.clear();
        match reader.read_line(&mut header) {
            Ok(0) => return None,
            Ok(_) => {
                if header == "\r\n" || header == "\n" {
                    break;
                }
            }
            Err(_) => return None,
        }
    }
    let mut body = String::new();
    let _ = reader.read_to_string(&mut body);
    let d: Value = serde_json::from_str(body.trim()).ok()?;
    d.pointer("/choices/0/message/content")
        .and_then(|x| x.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Join native llama tool_calls into one JSON object per call (cap 4).
pub fn synth_tool_calls(calls: &[(String, String)]) -> String {
    let mut parts = Vec::new();
    for (name, args) in calls.iter().take(4) {
        if name.is_empty() {
            continue;
        }
        let args_v: Value = serde_json::from_str(args).unwrap_or(json!({}));
        parts.push(json!({"name": name, "arguments": args_v}).to_string());
    }
    parts.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_200_is_ready_503_is_not() {
        assert!(health_line_ok("HTTP/1.1 200 OK"));
        assert!(!health_line_ok("HTTP/1.1 503 Service Unavailable"));
        assert!(!health_line_ok("HTTP/1.1 404 Not Found"));
    }

    #[test]
    fn invalid_endpoint_never_falls_back_to_default() {
        assert_eq!(endpoint("not-an-address", Some(8080)), None);
        assert_eq!(endpoint("127.0.0.1", None), None);
        assert_eq!(
            endpoint("127.0.0.1", Some(9000)),
            Some("127.0.0.1:9000".parse().unwrap())
        );
    }

    #[test]
    fn chat_payload_omits_openai_tools() {
        let tools = json!([{"type":"function","function":{"name":"list_dir"}}]);
        let body = chat_payload(&[json!({"role":"user","content":"hi"})], Some(tools));
        assert!(body.get("tools").is_none(), "{body}");
        assert!(body.get("tool_choice").is_none(), "{body}");
        assert_eq!(body["stream"], true);
    }

    #[test]
    fn synth_keeps_every_native_call() {
        let s = synth_tool_calls(&[
            ("list_dir".into(), r#"{"path":"~/pixie"}"#.into()),
            (
                "read_local_file".into(),
                r#"{"path":"~/pixie/README.md"}"#.into(),
            ),
        ]);
        assert!(s.contains("list_dir"), "{s}");
        assert!(s.contains("read_local_file"), "{s}");
        let n = crate::mind::extract_tool_calls(&s, 8);
        assert_eq!(n.len(), 2, "{s}");
    }
}

#[allow(dead_code)]
fn _read_all(s: &mut TcpStream) -> String {
    let mut b = String::new();
    let _ = s.read_to_string(&mut b);
    b
}
