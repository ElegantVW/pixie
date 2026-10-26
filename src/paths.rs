use anyhow::{bail, Result};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/dev/null"))
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

pub fn persona_path() -> PathBuf {
    config_dir().join("persona.md")
}

pub fn menagerie_path() -> PathBuf {
    config_dir().join("menagerie.json")
}

/// `$XDG_RUNTIME_DIR/pixie` (sockets, askpass wrapper). Not config, not cache.
pub fn runtime_dir() -> PathBuf {
    if let Ok(d) = std::env::var("XDG_RUNTIME_DIR") {
        let d = d.trim();
        if !d.is_empty() {
            return PathBuf::from(d).join("pixie");
        }
    }
    PathBuf::from(format!("/run/user/{}/pixie", unix_uid()))
}

fn unix_uid() -> u32 {
    #[cfg(unix)]
    {
        extern "C" {
            fn getuid() -> u32;
        }
        unsafe { getuid() }
    }
    #[cfg(not(unix))]
    {
        1000
    }
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
    let home = home
        .canonicalize()
        .map_err(|e| anyhow::anyhow!("cannot resolve home directory {}: {e}", home.display()))?;
    let p = resolve_for_write(&expanded)?;
    if !p.starts_with(&home) {
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
    let h = configured_home(std::env::var_os("HOME"))?;
    let blocked = blocked_prefixes(&h);
    assert_allowed_in(path, &h, &blocked)
}

fn configured_home(value: Option<std::ffi::OsString>) -> Result<PathBuf> {
    value
        .map(PathBuf::from)
        .filter(|home| home.is_absolute())
        .ok_or_else(|| anyhow::anyhow!("HOME must be set to an absolute path"))
}

pub fn write_atomic(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    use std::io::Write;

    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let (temp_path, mut temp_file) = loop {
        let sequence = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let candidate = parent.join(format!(".{name}.{}.{}.tmp", std::process::id(), sequence));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => break (candidate, file),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    };
    let result = (|| {
        temp_file.write_all(contents)?;
        temp_file.sync_all()?;
        drop(temp_file);
        std::fs::rename(&temp_path, path)?;
        std::fs::File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temp_path);
    }
    result
}

fn resolve_for_write(path: &Path) -> Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            std::path::Component::RootDir => normalized.push(component.as_os_str()),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            std::path::Component::Normal(part) => normalized.push(part),
            std::path::Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
        }
    }
    let path = normalized.as_path();
    let mut unresolved = Vec::new();
    let mut ancestor = path;
    let mut resolved = loop {
        match ancestor.canonicalize() {
            Ok(resolved) => break resolved,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if let Ok(meta) = std::fs::symlink_metadata(ancestor) {
                    if meta.file_type().is_symlink() {
                        bail!("cannot safely resolve symlink: {}", ancestor.display());
                    }
                }
                let name = ancestor
                    .file_name()
                    .ok_or_else(|| anyhow::anyhow!("cannot resolve path: {}", path.display()))?;
                unresolved.push(name.to_os_string());
                ancestor = ancestor
                    .parent()
                    .ok_or_else(|| anyhow::anyhow!("cannot resolve path: {}", path.display()))?;
            }
            Err(error) => {
                bail!("cannot safely resolve path {}: {error}", ancestor.display());
            }
        }
    };
    for part in unresolved.iter().rev() {
        resolved.push(part);
    }
    Ok(resolved)
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
    let extra = format!(
        "{}:{}",
        home().join("bin").display(),
        home().join(".local/bin").display()
    );
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
        assert!(
            assert_allowed_in(&format!("{}/.ssh/id_rsa", tmp.display()), &tmp, &blocked).is_err()
        );
        let ok = tmp.join("notes.txt");
        fs::write(&ok, "hi").unwrap();
        assert!(assert_allowed_in(ok.to_str().unwrap(), &tmp, &blocked).is_ok());
        let _ = fs::remove_dir_all(&tmp);
    }

    #[cfg(unix)]
    #[test]
    fn refuses_missing_descendant_under_external_symlink() {
        use std::os::unix::fs::symlink;

        let root = std::env::temp_dir().join(format!("pixie-path-{}", std::process::id()));
        let home = root.join("home");
        let outside = root.join("outside");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&outside).unwrap();
        symlink(&outside, home.join("linked")).unwrap();
        let target = home.join("linked/new/note.txt");
        assert!(assert_allowed_in(target.to_str().unwrap(), &home, &[]).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn refuses_dangling_symlink_path() {
        use std::os::unix::fs::symlink;

        let root = std::env::temp_dir().join(format!("pixie-dangling-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        symlink(root.join("outside"), root.join("link")).unwrap();
        assert!(assert_allowed_in(root.join("link/note").to_str().unwrap(), &root, &[]).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn refuses_missing_or_relative_home() {
        assert!(configured_home(None).is_err());
        assert!(configured_home(Some("relative-home".into())).is_err());
    }

    #[test]
    fn normalizes_parent_components_before_home_check() {
        let root = std::env::temp_dir().join(format!("pixie-parent-{}", std::process::id()));
        let home = root.join("home");
        fs::create_dir_all(&home).unwrap();
        let target = home.join("missing/../../outside/note");
        assert!(assert_allowed_in(target.to_str().unwrap(), &home, &[]).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
