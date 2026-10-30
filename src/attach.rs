//! User file clips. Path-gated. Images / UTF-8 text / honest "later" kinds.

use crate::diff::{FileKind, FileTouch};
use crate::paths;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub const TEXT_CAP: usize = 24_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Image,
    Text,
    Other,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Image => "image",
            Kind::Text => "text",
            Kind::Other => "other",
        }
    }

    fn from_str(s: &str) -> Self {
        match s {
            "image" => Kind::Image,
            "text" => Kind::Text,
            _ => Kind::Other,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attach {
    pub path: String,
    pub kind: Kind,
    pub bytes: u64,
    pub caption: Option<String>,
}

impl Attach {
    pub fn from_path(path: &Path) -> Result<Self, String> {
        let p = paths::assert_allowed(&path.display().to_string()).map_err(|e| e.to_string())?;
        if !p.is_file() {
            return Err(format!("not a file: {}", p.display()));
        }
        let bytes = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
        Ok(Self {
            path: p.display().to_string(),
            kind: classify(&p),
            bytes,
            caption: None,
        })
    }

    pub fn gone(&self) -> bool {
        !Path::new(&self.path).is_file()
    }

    pub fn excerpt(&self, cap: usize) -> Option<String> {
        if self.kind != Kind::Text {
            return None;
        }
        let bytes = std::fs::read(&self.path).ok()?;
        if bytes.contains(&0) || std::str::from_utf8(&bytes).is_err() {
            return None;
        }
        let mut t = String::from_utf8_lossy(&bytes).into_owned();
        if t.len() > cap {
            t = t.chars().take(cap).collect();
            t.push('…');
        }
        Some(t)
    }

    pub fn to_json(&self) -> Value {
        let mut v = json!({
            "path": self.path,
            "kind": self.kind.as_str(),
            "bytes": self.bytes,
        });
        if let Some(c) = &self.caption {
            v["caption"] = json!(c);
        }
        v
    }

    pub fn from_json(v: &Value) -> Option<Self> {
        let path = v.get("path").and_then(|x| x.as_str())?;
        if path.is_empty() {
            return None;
        }
        let kind = v
            .get("kind")
            .and_then(|x| x.as_str())
            .map(Kind::from_str)
            .unwrap_or_else(|| classify(Path::new(path)));
        let bytes = v.get("bytes").and_then(|x| x.as_u64()).unwrap_or(0);
        let caption = v
            .get("caption")
            .and_then(|x| x.as_str())
            .map(|s| s.to_string());
        Some(Self {
            path: path.into(),
            kind,
            bytes,
            caption,
        })
    }

    pub fn as_touch(&self) -> FileTouch {
        FileTouch {
            path: self.path.clone(),
            kind: match self.kind {
                Kind::Image => FileKind::Image,
                _ => FileKind::Read,
            },
            added: 0,
            deleted: 0,
            lines: Vec::new(),
            binary: self.kind != Kind::Text,
            extra_paths: Vec::new(),
            collapsed: false,
        }
    }
}

pub fn classify(path: &Path) -> Kind {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if matches!(
        ext.as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp"
    ) {
        return Kind::Image;
    }
    if let Ok(bytes) = std::fs::read(path) {
        if !bytes.contains(&0) && std::str::from_utf8(&bytes).is_ok() {
            return Kind::Text;
        }
    }
    Kind::Other
}

/// Bracketed paste of a single home path (or file:// URI) becomes a clip.
pub fn from_paste(s: &str) -> Option<Attach> {
    let t = s.trim();
    if t.is_empty() || t.contains('\n') || t.contains('\r') {
        return None;
    }
    let t = t.strip_prefix("file://").unwrap_or(t);
    if !(t.starts_with('/') || t.starts_with('~') || t.starts_with("~/")) {
        return None;
    }
    let p = paths::assert_allowed(t).ok()?;
    if !p.is_file() {
        return None;
    }
    Attach::from_path(&p).ok()
}

pub fn spellbook_bin() -> PathBuf {
    if let Ok(p) = std::env::var("SPELLBOOK") {
        return PathBuf::from(p);
    }
    let home = paths::home();
    for c in [
        home.join("bin/spellbook"),
        home.join(".local/lib/faeos/spellbook"),
        PathBuf::from("spellbook"),
    ] {
        if c.as_os_str() == "spellbook" || c.is_file() {
            return c;
        }
    }
    PathBuf::from("spellbook")
}

pub fn pick_start(pending: &[Attach]) -> PathBuf {
    if let Some(a) = pending.last() {
        if let Some(d) = Path::new(&a.path).parent() {
            if d.is_dir() {
                return d.to_path_buf();
            }
        }
    }
    let cwd = std::env::current_dir().unwrap_or_else(|_| paths::home());
    if paths::assert_allowed(&cwd.display().to_string()).is_ok() {
        cwd
    } else {
        paths::home()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn paste_home_file_attaches() {
        let dir = paths::home().join(format!(".cache/pixie/attach-test-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let p = dir.join("note.txt");
        fs::write(&p, "hello clip").unwrap();
        let a = from_paste(&p.display().to_string()).expect("paste");
        assert_eq!(a.kind, Kind::Text);
        assert!(a.excerpt(80).unwrap().contains("hello clip"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn paste_ssh_is_refused() {
        assert!(from_paste("/home/evenweaker/.ssh/id_rsa").is_none()
            || from_paste("~/.ssh/id_rsa").is_none());
        let home = paths::home();
        let ssh = home.join(".ssh/id_rsa");
        if ssh.is_file() {
            assert!(from_paste(&ssh.display().to_string()).is_none());
        }
        assert!(from_paste("not a path").is_none());
        assert!(from_paste("two\nlines").is_none());
    }

    #[test]
    fn image_ext_is_image_even_if_missing() {
        assert_eq!(classify(Path::new("/tmp/fair.png")), Kind::Image);
        assert_eq!(classify(Path::new("/tmp/x.pdf")), Kind::Other);
    }

    #[test]
    fn json_roundtrip() {
        let a = Attach {
            path: "/tmp/a.png".into(),
            kind: Kind::Image,
            bytes: 12,
            caption: Some("a pink fairy".into()),
        };
        let b = Attach::from_json(&a.to_json()).unwrap();
        assert_eq!(a, b);
    }
}
