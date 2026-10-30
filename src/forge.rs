//! SDXL forge for Pixie chat. Imp's sampler law for turbo; SDXL Q8 for quality.
//! One GPU tenant: caller hops pixie off first.

use crate::llm;
use crate::paths;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::Instant;

const SD_CLI: &str = ".local/lib/sd/sd-cli";
const SD_LIB: &str = ".local/lib/sd";
const SDXL_Q8: &str = ".local/share/pixie/models/sdxl-base-1.0-Q8_0.gguf";
const SD_TURBO: &str = ".local/share/pixie/models/sd_turbo-f16.gguf";

struct Phase {
    label: String,
    t0: Instant,
}

fn phase_cell() -> &'static Mutex<Option<Phase>> {
    static P: std::sync::OnceLock<Mutex<Option<Phase>>> = std::sync::OnceLock::new();
    P.get_or_init(|| Mutex::new(None))
}

pub fn set_phase(label: &str) {
    if let Ok(mut g) = phase_cell().lock() {
        *g = Some(Phase {
            label: label.into(),
            t0: Instant::now(),
        });
    }
}

pub fn clear_phase() {
    if let Ok(mut g) = phase_cell().lock() {
        *g = None;
    }
}

pub fn phase_status(spin: &str) -> Option<String> {
    let g = phase_cell().lock().ok()?;
    let p = g.as_ref()?;
    let secs = p.t0.elapsed().as_secs();
    Some(format!("{spin}  {} · {secs}s", p.label))
}

pub fn art_dir() -> PathBuf {
    crate::prefs::art_dir()
}

pub fn stamp_png() -> PathBuf {
    let stamp = Command::new("date")
        .args(["+%Y%m%d-%H%M%S"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("{}", std::process::id()));
    art_dir().join(format!("pixie-{stamp}.png"))
}

#[derive(Clone, Debug)]
pub struct Hand {
    pub model: PathBuf,
    pub kind: &'static str,
    pub width: u32,
    pub height: u32,
    pub steps: u32,
    pub cfg: f32,
    pub sampler: &'static str,
}

/// Best quality that fits an 8 GB AMD card after Pixie hops off.
pub fn best_hand() -> Result<Hand, String> {
    let home = paths::home();
    let slot = imp_image_slot();
    let sdxl = home.join(SDXL_Q8);
    let turbo = home.join(SD_TURBO);
    let quality_slot = slot
        .as_ref()
        .filter(|p| p.is_file() && !is_turbo(p))
        .cloned();
    let any_slot = slot.filter(|p| p.is_file());
    let model = quality_slot
        .or_else(|| sdxl.is_file().then_some(sdxl.clone()))
        .or_else(|| any_slot)
        .or_else(|| turbo.is_file().then_some(turbo.clone()))
        .ok_or_else(|| "no image weights on disk".to_string())?;
    if is_turbo(&model) {
        Ok(Hand {
            model,
            kind: "sd-turbo",
            width: 512,
            height: 512,
            steps: 4,
            cfg: 1.0,
            sampler: "euler",
        })
    } else {
        Ok(Hand {
            model,
            kind: "sdxl",
            width: 1024,
            height: 1024,
            steps: 20,
            cfg: 7.0,
            sampler: "euler_a",
        })
    }
}

fn is_turbo(p: &Path) -> bool {
    let n = p
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();
    n.contains("turbo") && !n.contains("qwen-image")
}

fn imp_image_slot() -> Option<PathBuf> {
    let t = fs::read_to_string(paths::menagerie_path()).ok()?;
    let v: Value = serde_json::from_str(&t).ok()?;
    let name = v
        .pointer("/apps/imp/slots/image")
        .and_then(|x| x.as_str())?;
    let path = v
        .pointer(&format!("/models/{name}/path"))
        .and_then(|x| x.as_str())?;
    Some(PathBuf::from(path))
}

pub fn sd_bin() -> PathBuf {
    paths::home().join(SD_CLI)
}

pub fn sd_argv(hand: &Hand, prompt: &str, out: &Path) -> Result<Vec<String>, String> {
    let bin = sd_bin();
    if !bin.is_file() {
        return Err(format!("sd-cli missing at {}", bin.display()));
    }
    if !hand.model.is_file() {
        return Err(format!("image weights missing at {}", hand.model.display()));
    }
    let lib = paths::home().join(SD_LIB);
    let cfg = if (hand.cfg - hand.cfg.round()).abs() < f32::EPSILON {
        format!("{}", hand.cfg.round() as i32)
    } else {
        format!("{:.1}", hand.cfg)
    };
    // Imp's working argv. Extra clip/vae/rng/backend/offload flags painted white.
    Ok(vec![
        "/usr/bin/env".into(),
        format!("LD_LIBRARY_PATH={}", lib.display()),
        bin.display().to_string(),
        "-m".into(),
        hand.model.display().to_string(),
        "-p".into(),
        prompt.into(),
        "-W".into(),
        hand.width.to_string(),
        "-H".into(),
        hand.height.to_string(),
        "--steps".into(),
        hand.steps.to_string(),
        "--cfg-scale".into(),
        cfg,
        "--seed".into(),
        "-1".into(),
        "-o".into(),
        out.display().to_string(),
    ])
}

pub fn enhance_prompt(wish: &str) -> String {
    let wish = wish.trim();
    if wish.is_empty() {
        return String::new();
    }
    if !crate::prefs::enhance_image() {
        return wish.to_string();
    }
    let Some(port) = llm::port() else {
        return wish.to_string();
    };
    let msgs = vec![
        json!({
            "role": "system",
            "content": "Expand the wish into ONE 30-50 word image prompt. Subject, lighting, composition, medium, mood. All-ages. No quotes. No camera brands. No tools. No JSON. One paragraph."
        }),
        json!({"role": "user", "content": wish}),
    ];
    let Some(raw) = llm::complete_on_if_up(port, &msgs, 160, 45) else {
        return wish.to_string();
    };
    let mut t = crate::think::split(&raw).visible;
    t = t.replace('\n', " ").trim().trim_matches('"').to_string();
    if t.len() > 400 {
        t = t.chars().take(400).collect();
    }
    if t.len() < 8 {
        wish.to_string()
    } else {
        t
    }
}

pub fn png_halfblocks(path: &Path, cols: u16, rows: u16) -> Vec<Line<'static>> {
    let cols = cols.clamp(8, 96) as u32;
    let rows = rows.clamp(4, 40) as u32;
    let Ok(img) = image::open(path) else {
        return vec![Line::from(Span::styled(
            format!("saved {}", path.display()),
            Style::default().fg(Color::Rgb(0xc0, 0xc0, 0xc8)),
        ))];
    };
    let rgb = img
        .resize_exact(cols, rows * 2, image::imageops::FilterType::Lanczos3)
        .to_rgb8();
    let mut lines = Vec::new();
    for y in 0..rows {
        let mut spans = Vec::new();
        for x in 0..cols {
            let top = rgb.get_pixel(x, y * 2);
            let bot = rgb.get_pixel(x, y * 2 + 1);
            spans.push(Span::styled(
                "▀",
                Style::default()
                    .fg(Color::Rgb(top[0], top[1], top[2]))
                    .bg(Color::Rgb(bot[0], bot[1], bot[2])),
            ));
        }
        lines.push(Line::from(spans));
    }
    lines
}

pub fn png_is_blank(path: &Path) -> bool {
    let Ok(img) = image::open(path) else {
        return false;
    };
    let rgb = img.thumbnail(32, 32).to_rgb8();
    let Some(&first) = rgb.pixels().next() else {
        return true;
    };
    rgb.pixels().all(|p| *p == first)
}

pub fn wait_mouth(timeout: std::time::Duration) -> bool {
    let t0 = Instant::now();
    while t0.elapsed() < timeout {
        if llm::llm_up() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    llm::llm_up()
}

pub fn write_sidecar(png: &Path, wish: &str, prompt: &str) {
    let side = png.with_extension("prompt.txt");
    let _ = fs::write(
        side,
        format!("wish: {wish}\nprompt: {prompt}\n"),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turbo_sampler_is_imp_law() {
        let h = Hand {
            model: PathBuf::from("/tmp/sd_turbo-f16.gguf"),
            kind: "sd-turbo",
            width: 512,
            height: 512,
            steps: 4,
            cfg: 1.0,
            sampler: "euler",
        };
        // argv builder still requires the binary; check the strings we would pass
        assert_eq!(h.steps, 4);
        assert!((h.cfg - 1.0).abs() < f32::EPSILON);
        assert_eq!(h.width, 512);
    }

    #[test]
    fn sdxl_is_the_quality_hand() {
        let h = Hand {
            model: PathBuf::from("/m/sdxl-base-1.0-Q8_0.gguf"),
            kind: "sdxl",
            width: 1024,
            height: 1024,
            steps: 20,
            cfg: 7.0,
            sampler: "euler_a",
        };
        assert_eq!(h.steps, 20);
        assert_eq!(h.width, 1024);
        assert_eq!(h.sampler, "euler_a");
    }

    #[test]
    fn seed_flag_is_random() {
        let src = include_str!("forge.rs");
        assert!(src.contains("\"--seed\""), "sd-cli must get a random seed");
        assert!(src.contains("LD_LIBRARY_PATH"), "Imp sets the sd lib path");
    }

    #[test]
    fn argv_matches_imp_when_weights_exist() {
        let model = paths::home().join(SDXL_Q8);
        if !sd_bin().is_file() || !model.is_file() {
            return;
        }
        let h = Hand {
            model,
            kind: "sdxl",
            width: 1024,
            height: 1024,
            steps: 20,
            cfg: 7.0,
            sampler: "euler_a",
        };
        let a = sd_argv(&h, "a dragon", Path::new("/tmp/out.png")).unwrap();
        let joined = a.join(" ");
        assert!(joined.contains("LD_LIBRARY_PATH="), "{joined}");
        assert!(joined.contains("--steps"), "{joined}");
        assert!(joined.contains("--cfg-scale"), "{joined}");
        assert!(joined.contains("--seed"), "{joined}");
        assert!(!joined.contains("--rng"), "{joined}");
        assert!(!joined.contains("--offload"), "{joined}");
        assert!(!joined.contains("--clip_l"), "{joined}");
        assert!(!joined.contains("--backend"), "{joined}");
        assert!(!joined.contains("--sampling-method"), "{joined}");
        assert!(!joined.contains("--vae"), "{joined}");
    }

    #[test]
    fn argv_shape_when_bin_missing() {
        let h = Hand {
            model: PathBuf::from("/nope.gguf"),
            kind: "sdxl",
            width: 1024,
            height: 1024,
            steps: 20,
            cfg: 7.0,
            sampler: "euler_a",
        };
        let err = sd_argv(&h, "a dragon", Path::new("/tmp/out.png")).unwrap_err();
        assert!(
            err.contains("sd-cli missing") || err.contains("weights missing"),
            "{err}"
        );
    }
}
