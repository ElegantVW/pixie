use anyhow::{bail, Result};
use std::path::{Path, PathBuf};

pub fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

pub fn config_dir() -> PathBuf {
    home().join(".config/pixie")
}

pub fn data_dir() -> PathBuf {
    home().join(".local/share/pixie")
}

pub fn cache_dir() -> PathBuf {
    home().join(".cache/pixie")
}

pub fn memory_path() -> PathBuf {
    data_dir().join("memory.md")
}

pub fn sessions_dir() -> PathBuf {
    cache_dir().join("sessions")
}

pub fn plans_dir() -> PathBuf {
    cache_dir().join("plans")
}

pub fn research_dir() -> PathBuf {
    cache_dir().join("research")
}

pub fn ui_path() -> PathBuf {
    config_dir().join("ui.json")
}

pub fn blocked_prefixes(home: &Path) -> Vec<PathBuf> {
    vec![
        home.join(".ssh"),
        home.join(".gnupg"),
        home.join(".grok").join("credentials"),
    ]
}

pub fn assert_allowed_in(path: &str, home: &Path, blocked: &[PathBuf]) -> Result<PathBuf> {
    let expanded = expand_user(path, home);
    let p = expanded.canonicalize().unwrap_or(expanded);
    if !p.starts_with(home) {
        bail!("refusing path outside home ({}): {path}", home.display());
    }
    for b in blocked {
        let b = b.canonicalize().unwrap_or_else(|_| b.clone());
        if p.starts_with(&b) {
            bail!("refusing sensitive path: {path}");
        }
    }
    Ok(p)
}

pub fn assert_allowed(path: &str) -> Result<PathBuf> {
    let h = home();
    let blocked = blocked_prefixes(&h);
    assert_allowed_in(path, &h, &blocked)
}

pub fn expand_user(path: &str, home: &Path) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        home.join(rest)
    } else if path == "~" {
        home.to_path_buf()
    } else {
        PathBuf::from(path)
    }
}

pub fn house_path() -> String {
    let extra = format!("{}:{}", home().join("bin").display(), home().join(".local/bin").display());
    match std::env::var("PATH") {
        Ok(p) => format!("{extra}:{p}"),
        Err(_) => extra,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn refuses_etc_and_ssh() {
        let tmp = std::env::temp_dir().join(format!("pixie-home-{}", std::process::id()));
        let _ = fs::create_dir_all(tmp.join(".ssh"));
        let blocked = blocked_prefixes(&tmp);
        assert!(assert_allowed_in("/etc/passwd", &tmp, &blocked).is_err());
        assert!(assert_allowed_in(&format!("{}/.ssh/id_rsa", tmp.display()), &tmp, &blocked).is_err());
        let ok = tmp.join("notes.txt");
        fs::write(&ok, "hi").unwrap();
        assert!(assert_allowed_in(ok.to_str().unwrap(), &tmp, &blocked).is_ok());
    }
}
