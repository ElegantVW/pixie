//! Probe the machine so Pixie does not invent Ubuntu/`apt` on an Arch box.

use std::fs;
use std::path::Path;
use std::process::Command;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OsId {
    pub name: String,
    pub id: String,
    pub like: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pkg {
    Pacman,
    Apk,
    Xbps,
    Dnf,
    Zypper,
    Apt,
    Brew,
    Pkg,
    Unknown,
}

impl Pkg {
    pub fn name(self) -> &'static str {
        match self {
            Pkg::Pacman => "pacman",
            Pkg::Apk => "apk",
            Pkg::Xbps => "xbps-install",
            Pkg::Dnf => "dnf",
            Pkg::Zypper => "zypper",
            Pkg::Apt => "apt-get",
            Pkg::Brew => "brew",
            Pkg::Pkg => "pkg",
            Pkg::Unknown => "unknown",
        }
    }

    pub fn forbidden(self) -> &'static [&'static str] {
        match self {
            Pkg::Pacman => &["apt", "apt-get", "yum", "dnf"],
            Pkg::Apt => &["pacman", "yum", "dnf", "apk"],
            Pkg::Dnf => &["pacman", "apt", "apt-get"],
            Pkg::Apk => &["pacman", "apt", "apt-get"],
            Pkg::Brew => &["pacman", "apt-get"],
            Pkg::Zypper => &["pacman", "apt-get"],
            Pkg::Xbps => &["pacman", "apt-get"],
            Pkg::Pkg => &["pacman", "apt-get"],
            Pkg::Unknown => &[],
        }
    }
}

pub fn from_os_release(text: &str) -> OsId {
    let mut name = String::new();
    let mut id = String::new();
    let mut like = String::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("NAME=") {
            name = unquote(rest);
        } else if let Some(rest) = line.strip_prefix("ID=") {
            id = unquote(rest);
        } else if let Some(rest) = line.strip_prefix("ID_LIKE=") {
            like = unquote(rest);
        }
    }
    if name.is_empty() {
        name = "unknown".into();
    }
    OsId { name, id, like }
}

fn unquote(s: &str) -> String {
    s.trim().trim_matches('"').trim_matches('\'').to_string()
}

pub fn pick_pkg(which_names: &[&str]) -> Pkg {
    for n in which_names {
        match *n {
            "pacman" => return Pkg::Pacman,
            "apk" => return Pkg::Apk,
            "xbps-install" => return Pkg::Xbps,
            "dnf" => return Pkg::Dnf,
            "zypper" => return Pkg::Zypper,
            "apt-get" => return Pkg::Apt,
            "brew" => return Pkg::Brew,
            "pkg" => return Pkg::Pkg,
            _ => {}
        }
    }
    Pkg::Unknown
}

const PKG_ORDER: [&str; 8] = [
    "pacman",
    "apk",
    "xbps-install",
    "dnf",
    "zypper",
    "apt-get",
    "brew",
    "pkg",
];

pub fn card_text(
    os: &OsId,
    kernel: &str,
    hostname: &str,
    pkg: Pkg,
    shell: &str,
    ram_kb: u64,
    cpu: &str,
    cores: u32,
    gpu: &str,
    term: &str,
    session: &str,
) -> String {
    let mut lines = vec![
        format!("host: {hostname}"),
        format!("os: {} (id={})", os.name, os.id),
        format!("kernel: {kernel}"),
        format!(
            "shell: {shell}  (this box: zsh on Arch — do not assume bashisms beyond POSIX / sh -c)"
        ),
        format!("term: {term}"),
        format!("session: {session}"),
        format!("house PATH: ~/bin:~/.local/bin"),
    ];
    if !cpu.is_empty() {
        if cores > 0 {
            lines.push(format!("cpu: {cpu} ({cores} threads)"));
        } else {
            lines.push(format!("cpu: {cpu}"));
        }
    }
    if !gpu.is_empty() {
        lines.push(format!("gpu: {gpu}"));
    }
    if ram_kb > 0 {
        lines.push(format!("ram: {:.0} GiB", ram_kb as f64 / 1024.0 / 1024.0));
    }
    match pkg {
        Pkg::Pacman => lines.push(
            "packages: pacman (alchemy brew/sip/distill) — never apt/apt-get on this box".into(),
        ),
        Pkg::Unknown => lines.push("packages: unknown — ask before installing".into()),
        other => {
            let forbid = other.forbidden();
            if forbid.is_empty() {
                lines.push(format!("packages: {}", other.name()));
            } else {
                lines.push(format!(
                    "packages: {} — never {}",
                    other.name(),
                    forbid.join("/")
                ));
            }
        }
    }
    if !os.like.is_empty() {
        lines.push(format!("id_like: {}", os.like));
    }
    lines.push(
        "one GPU llama at a time on 8 GB class cards (pixie 8080; kur/smolLM 8081 CPU)".into(),
    );
    let mut s = lines.join("\n");
    if s.len() > 1400 {
        s.truncate(1400);
        s.push('…');
    }
    s
}

pub fn probe() -> String {
    let os = fs::read_to_string("/etc/os-release")
        .ok()
        .map(|t| from_os_release(&t))
        .unwrap_or(OsId {
            name: "unknown".into(),
            id: String::new(),
            like: String::new(),
        });
    let kernel = uname(&["-srm"]);
    let hostname = uname(&["-n"]);
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "?".into());
    let present: Vec<&str> = PKG_ORDER.iter().copied().filter(|n| which(n)).collect();
    let pkg = pick_pkg(&present);
    let ram_kb = mem_total_kb();
    let cpu = cpu_model();
    let cores = cpu_threads();
    let gpu = gpu_name();
    let term = std::env::var("TERM").unwrap_or_else(|_| "?".into());
    let session = std::env::var("XDG_SESSION_TYPE")
        .or_else(|_| std::env::var("DESKTOP_SESSION"))
        .unwrap_or_else(|_| "?".into());
    card_text(
        &os, &kernel, &hostname, pkg, &shell, ram_kb, &cpu, cores, &gpu, &term, &session,
    )
}

fn uname(args: &[&str]) -> String {
    Command::new("uname")
        .args(args)
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

fn which(name: &str) -> bool {
    let path = std::env::var("PATH").unwrap_or_default();
    for dir in path.split(':') {
        if dir.is_empty() {
            continue;
        }
        let p = Path::new(dir).join(name);
        if p.is_file() {
            return true;
        }
    }
    false
}

fn cpu_model() -> String {
    let Ok(text) = fs::read_to_string("/proc/cpuinfo") else {
        return String::new();
    };
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("model name") {
            return rest.trim_start_matches(':').trim().to_string();
        }
    }
    String::new()
}

fn cpu_threads() -> u32 {
    fs::read_to_string("/proc/cpuinfo")
        .ok()
        .map(|t| t.lines().filter(|l| l.starts_with("processor")).count() as u32)
        .unwrap_or(0)
}

fn gpu_name() -> String {
    let drm = Path::new("/sys/class/drm");
    let Ok(rd) = fs::read_dir(drm) else {
        return String::new();
    };
    for ent in rd.flatten() {
        let name = ent.file_name();
        let n = name.to_string_lossy();
        if !n.starts_with("card") || n.contains('-') {
            continue;
        }
        let p = ent.path().join("device/uevent");
        if let Ok(t) = fs::read_to_string(p) {
            for line in t.lines() {
                if let Some(rest) = line.strip_prefix("DRIVER=") {
                    return rest.trim().to_string();
                }
            }
        }
    }
    String::new()
}

fn mem_total_kb() -> u64 {
    let Ok(text) = fs::read_to_string("/proc/meminfo") else {
        return 0;
    };
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("MemTotal:") {
            let num = rest.split_whitespace().next().unwrap_or("0");
            return num.parse().unwrap_or(0);
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    const ARCH: &str = r#"NAME="Arch Linux"
ID=arch
BUILD_ID=rolling
"#;

    const DEBIAN: &str = r#"NAME="Debian GNU/Linux"
ID=debian
ID_LIKE=debian
VERSION_ID="12"
"#;

    #[test]
    fn arch_card_forbids_apt() {
        let os = from_os_release(ARCH);
        assert_eq!(os.id, "arch");
        let pkg = pick_pkg(&["pacman", "apt-get"]);
        assert_eq!(pkg, Pkg::Pacman);
        let card = card_text(
            &os,
            "Linux 6.12 x86_64",
            "VANGUARDA",
            pkg,
            "/usr/bin/zsh",
            64 * 1024 * 1024,
            "AMD Ryzen",
            16,
            "amdgpu",
            "xterm-kitty",
            "x11",
        );
        assert!(card.contains("Arch"), "{card}");
        assert!(card.contains("pacman"), "{card}");
        assert!(card.contains("never apt"), "{card}");
        assert!(card.contains("zsh"), "{card}");
        assert!(card.contains("amdgpu") || card.contains("AMD"), "{card}");
        assert!(!card.contains("never pacman"), "{card}");
    }

    #[test]
    fn debian_card_allows_apt() {
        let os = from_os_release(DEBIAN);
        let pkg = pick_pkg(&["apt-get"]);
        assert_eq!(pkg, Pkg::Apt);
        let card = card_text(
            &os,
            "Linux 6.1 x86_64",
            "box",
            pkg,
            "/bin/bash",
            8 * 1024 * 1024,
            "",
            0,
            "",
            "xterm",
            "tty",
        );
        assert!(card.contains("Debian"), "{card}");
        assert!(card.contains("apt-get"), "{card}");
        assert!(card.contains("never pacman"), "{card}");
        assert!(!card.contains("alchemy"), "{card}");
    }
}
