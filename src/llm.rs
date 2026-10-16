//! Loopback llama-server client. Abort = drop the socket.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub fn port() -> u16 {
    std::env::var("PIXIE_LLM_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8080)
}

pub fn host() -> String {
    std::env::var("PIXIE_LLM_HOST").unwrap_or_else(|_| "127.0.0.1".into())
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
    let addr = format!("{}:{}", host(), port());
    let Ok(mut s) = TcpStream::connect_timeout(
        &addr.parse().unwrap_or_else(|_| "127.0.0.1:8080".parse().unwrap()),
        Duration::from_secs(2),
    ) else {
        return false;
    };
    let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
    let req = format!(
        "GET /health HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n"
    );
    if s.write_all(req.as_bytes()).is_err() {
        return false;
    }
    let mut buf = String::new();
    let mut r = BufReader::new(s);
    if r.read_line(&mut buf).is_err() {
        return false;
    }
    buf.contains("200") || buf.contains("503")
}

pub fn ensure_llm_up() -> bool {
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
    let mut body = json!({
        "messages": messages,
        "temperature": 0.2,
        "max_tokens": 2048,
        "stream": true,
    });
    if let Some(t) = tools {
        body["tools"] = t;
        body["tool_choice"] = json!("auto");
    }
    let payload = body.to_string();
    let addr = format!("{}:{}", host(), port());
    let sock_addr: std::net::SocketAddr = match addr.parse() {
        Ok(a) => a,
        Err(_) => {
            return StreamOut {
                text: String::new(),
                err: Some(format!("bad llm addr {addr}")),
            };
        }
    };
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
    let mut acc_tools: std::collections::BTreeMap<i64, (String, String)> = std::collections::BTreeMap::new();
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
        for ch in d.get("choices").and_then(|x| x.as_array()).cloned().unwrap_or_default() {
            let delta = ch.get("delta").cloned().unwrap_or(json!({}));
            if let Some(tcs) = delta.get("tool_calls").and_then(|x| x.as_array()) {
                for tc in tcs {
                    let idx = tc.get("index").and_then(|x| x.as_i64()).unwrap_or(0);
                    let slot = acc_tools.entry(idx).or_insert((String::new(), String::new()));
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
        if let Some((_, (name, args))) = acc_tools.iter().next() {
            let args_v: Value = serde_json::from_str(args).unwrap_or(json!({"command": args}));
            let synth = json!({"name": name, "arguments": args_v}).to_string();
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

#[allow(dead_code)]
fn _read_all(s: &mut TcpStream) -> String {
    let mut b = String::new();
    let _ = s.read_to_string(&mut b);
    b
}
