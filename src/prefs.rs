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
}

impl Default for UiPrefs {
    fn default() -> Self {
        Self {
            verbose: false,
            sparkle: true,
            rune: RuneKey::Chat,
            build_audit_rounds: true,
            sudo_less_safe: false,
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
    })
    .to_string();
    paths::write_atomic(&path, contents.as_bytes())
        .map_err(|e| anyhow::anyhow!("writing {}: {e}", path.display()))
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

pub fn apply_mouth(model: &str, backend: &str, ctx: u32) -> Result<(), String> {
    set_pixie_ctx(ctx);
    let men = menagerie_bin();
    if !model.is_empty() {
        run_men(&men, &["set", "pixie", model])?;
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

pub fn eye_asleep() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_prefs_sparkle_on() {
        let p = UiPrefs::default();
        assert!(p.sparkle);
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
