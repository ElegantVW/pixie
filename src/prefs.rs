//! House knobs: ui.json, persona overlay, menagerie registry.

use crate::paths;
use crate::runes::RuneKey;
use serde_json::{json, Value};
use std::fs;
use std::process::Command;

#[derive(Clone, Debug)]
pub struct UiPrefs {
    pub verbose: bool,
    pub sparkle: bool,
    pub rune: RuneKey,
    pub build_audit_rounds: bool,
    pub sudo_less_safe: bool,
    pub enhance_image: bool,
    pub art_dir: String,
}

impl Default for UiPrefs {
    fn default() -> Self {
        Self {
            verbose: false,
            sparkle: true,
            rune: RuneKey::Chat,
            build_audit_rounds: true,
            sudo_less_safe: false,
            enhance_image: true,
            art_dir: "~/pixie_art".into(),
        }
    }
}

pub fn load_ui() -> anyhow::Result<UiPrefs> {
    load_ui_from(&paths::ui_path())
}

fn load_ui_from(path: &std::path::Path) -> anyhow::Result<UiPrefs> {
    let t = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(UiPrefs::default()),
        Err(error) => return Err(anyhow::anyhow!("reading {}: {error}", path.display())),
    };
    let v: Value =
        serde_json::from_str(&t).map_err(|e| anyhow::anyhow!("parsing {}: {e}", path.display()))?;
    let rune = v
        .get("rune")
        .and_then(|x| x.as_str())
        .and_then(RuneKey::from_str)
        .unwrap_or(RuneKey::Chat);
    Ok(UiPrefs {
        verbose: v.get("verbose").and_then(|x| x.as_bool()).unwrap_or(false),
        sparkle: v.get("sparkle").and_then(|x| x.as_bool()).unwrap_or(true),
        rune,
        build_audit_rounds: v
            .get("build_audit_rounds")
            .and_then(|x| x.as_bool())
            .unwrap_or(true),
        sudo_less_safe: v
            .get("sudo_less_safe")
            .and_then(|x| x.as_bool())
            .unwrap_or(false),
        enhance_image: v
            .get("enhance_image")
            .and_then(|x| x.as_bool())
            .unwrap_or(true),
        art_dir: v
            .get("art_dir")
            .and_then(|x| x.as_str())
            .unwrap_or("~/pixie_art")
            .to_string(),
    })
}

pub fn save_ui(p: &UiPrefs) -> anyhow::Result<()> {
    let path = paths::ui_path();
    if let Some(d) = path.parent() {
        fs::create_dir_all(d).map_err(|e| anyhow::anyhow!("creating {}: {e}", d.display()))?;
    }
    let contents = json!({
        "verbose": p.verbose,
        "sparkle": p.sparkle,
        "rune": p.rune.as_str(),
        "build_audit_rounds": p.build_audit_rounds,
        "sudo_less_safe": p.sudo_less_safe,
        "enhance_image": p.enhance_image,
        "art_dir": p.art_dir,
    })
    .to_string();
    paths::write_atomic(&path, contents.as_bytes())
        .map_err(|e| anyhow::anyhow!("writing {}: {e}", path.display()))
}

pub fn enhance_image() -> bool {
    load_ui().map(|p| p.enhance_image).unwrap_or(true)
}

pub fn art_dir() -> std::path::PathBuf {
    let raw = load_ui()
        .map(|p| p.art_dir)
        .unwrap_or_else(|_| "~/pixie_art".into());
    let home = paths::home();
    let p = if let Some(rest) = raw.strip_prefix("~/") {
        home.join(rest)
    } else if raw == "~" {
        home.join("pixie_art")
    } else {
        std::path::PathBuf::from(raw)
    };
    paths::assert_allowed(&p.display().to_string()).unwrap_or_else(|_| home.join("pixie_art"))
}

pub fn persona_text() -> Option<String> {
    let t = fs::read_to_string(paths::persona_path()).ok()?;
    let t = t.trim().to_string();
    if t.is_empty() {
        None
    } else {
        Some(t)
    }
}

#[derive(Clone, Debug)]
pub struct Mouth {
    pub model: String,
    pub backend: String,
    pub ctx: u32,
}

pub fn pixie_mouth() -> Mouth {
    let v = read_reg();
    let app = v.pointer("/apps/pixie").cloned().unwrap_or(json!({}));
    Mouth {
        model: app
            .get("model")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        backend: app
            .get("backend")
            .and_then(|x| x.as_str())
            .unwrap_or("gpu")
            .to_string(),
        ctx: app.get("ctx").and_then(|x| x.as_u64()).unwrap_or(16384) as u32,
    }
}

pub fn model_names() -> Vec<String> {
    let v = read_reg();
    let mut names: Vec<String> = v
        .get("models")
        .and_then(|m| m.as_object())
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();
    names.sort();
    names
}

/// Text mouths only. Pony / SDXL stay Imp's hop.
pub fn chat_model_names() -> Vec<String> {
    model_names()
        .into_iter()
        .filter(|n| {
            let l = n.to_lowercase();
            !(l.contains("pony")
                || l.contains("sdxl")
                || l.contains("sd_turbo")
                || l.contains("sd-turbo")
                || l.starts_with("sd_"))
        })
        .collect()
}

pub fn set_pixie_ctx(ctx: u32) {
    let mut v = read_reg();
    if let Some(app) = v.pointer_mut("/apps/pixie") {
        if let Some(obj) = app.as_object_mut() {
            obj.insert("ctx".into(), json!(ctx));
        }
    }
    write_reg(&v);
}

#[allow(dead_code)]
pub fn apply_mouth(model: &str, backend: &str, ctx: u32) -> Result<(), String> {
    apply_mouth_slot("chat", model, backend, ctx)
}

pub fn apply_mouth_slot(slot: &str, model: &str, backend: &str, ctx: u32) -> Result<(), String> {
    if slot == "vision" && !mmproj_ready() {
        return Err("eye is sleeping — no mmproj on disk".into());
    }
    set_pixie_ctx(ctx);
    let men = menagerie_bin();
    if !model.is_empty() {
        if run_men(&men, &["set", "pixie", slot, model]).is_err() {
            run_men(&men, &["set", "pixie", model])?;
        }
    }
    if backend == "gpu" || backend == "cpu" {
        run_men(&men, &["backend", "pixie", backend])?;
    }
    let _ = Command::new(&men).args(["stop", "pixie"]).output();
    run_men(&men, &["ensure", "pixie"])?;
    Ok(())
}

fn menagerie_bin() -> std::path::PathBuf {
    let p = paths::home().join("bin/menagerie");
    if p.is_file() {
        p
    } else {
        std::path::PathBuf::from("menagerie")
    }
}

fn run_men(bin: &std::path::Path, args: &[&str]) -> Result<(), String> {
    let out = Command::new(bin)
        .args(args)
        .output()
        .map_err(|e| format!("menagerie: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        let err = err.trim();
        if err.is_empty() {
            Err(format!("menagerie {} failed", args.join(" ")))
        } else {
            Err(err.chars().take(160).collect())
        }
    }
}

fn read_reg() -> Value {
    fs::read_to_string(paths::menagerie_path())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(json!({}))
}

fn write_reg(v: &Value) {
    let p = paths::menagerie_path();
    if let Some(d) = p.parent() {
        let _ = fs::create_dir_all(d);
    }
    let _ = fs::write(
        p,
        serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string()),
    );
}

pub fn models_dir() -> std::path::PathBuf {
    paths::data_dir().join("models")
}

pub fn mmproj_file() -> Option<std::path::PathBuf> {
    let dir = models_dir();
    let rd = fs::read_dir(&dir).ok()?;
    let mut hits: Vec<std::path::PathBuf> = Vec::new();
    for e in rd.flatten() {
        let n = e.file_name().to_string_lossy().to_lowercase();
        if n.contains("mmproj") && n.ends_with(".gguf") && e.path().is_file() {
            if e.metadata().map(|m| m.len()).unwrap_or(0) > 1_000_000 {
                hits.push(e.path());
            }
        }
    }
    hits.sort_by_key(|p| {
        let n = p
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase();
        (!n.contains("f16"), n)
    });
    hits.into_iter().next()
}

pub fn mmproj_ready() -> bool {
    mmproj_file().is_some()
}

pub fn eye_asleep() -> bool {
    !mmproj_ready()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_prefs_sparkle_on() {
        let p = UiPrefs::default();
        assert!(p.sparkle);
        assert!(p.enhance_image);
        assert_eq!(p.art_dir, "~/pixie_art");
        assert_eq!(p.rune, RuneKey::Chat);
    }

    #[test]
    fn malformed_ui_preferences_are_reported() {
        let dir = std::env::temp_dir().join(format!("pixie-prefs-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ui.json");
        fs::write(&path, "{").unwrap();
        assert!(load_ui_from(&path).is_err());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn chat_models_drop_image_weights() {
        let names = [
            "pony-v6-Q8_0",
            "sdxl-base-1.0-Q8_0",
            "Huihui-Qwen3-8B-abliterated-v2.i1-Q4_K_M",
        ];
        let keep: Vec<_> = names
            .iter()
            .filter(|n| {
                let l = n.to_lowercase();
                !(l.contains("pony")
                    || l.contains("sdxl")
                    || l.contains("sd_turbo")
                    || l.contains("sd-turbo")
                    || l.starts_with("sd_"))
            })
            .copied()
            .collect();
        assert_eq!(keep, ["Huihui-Qwen3-8B-abliterated-v2.i1-Q4_K_M"]);
    }
}
