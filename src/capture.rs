//! Screenshot capture.
//!
//! linkshot grabs the **whole screen** up front (before its own overlay window is
//! shown), then does region selection + annotation inside that overlay — the
//! Lightshot / flameshot model. We shell out to whatever native screenshot tool is
//! available, because a reliable full-screen grab differs a lot between X11,
//! Wayland-wlroots, GNOME, KDE and macOS.

use anyhow::{anyhow, Context, Result};
use std::path::PathBuf;
use std::process::Command;

struct Backend {
    /// Binaries that must all exist on PATH for this backend to be usable.
    requires: &'static [&'static str],
    program: &'static str,
    args: &'static [&'static str],
    /// A shell pipeline (run via `sh -c`); when set, program/args are ignored.
    shell: Option<&'static str>,
}

/// Backends that grab the entire screen to `@OUT@` (a PNG path).
fn fullscreen_backends() -> Vec<Backend> {
    let mut v = Vec::new();

    #[cfg(target_os = "macos")]
    v.push(Backend {
        requires: &["screencapture"],
        program: "screencapture",
        args: &["-x", "-t", "png", "@OUT@"],
        shell: None,
    });

    #[cfg(target_os = "linux")]
    {
        let wayland = std::env::var("WAYLAND_DISPLAY").is_ok()
            || std::env::var("XDG_SESSION_TYPE").map(|s| s == "wayland").unwrap_or(false);
        if wayland {
            v.push(Backend { requires: &["grim"], program: "grim", args: &["@OUT@"], shell: None });
            v.push(Backend {
                requires: &["gnome-screenshot"],
                program: "gnome-screenshot",
                args: &["-f", "@OUT@"],
                shell: None,
            });
            v.push(Backend {
                requires: &["spectacle"],
                program: "spectacle",
                args: &["-b", "-n", "-f", "-o", "@OUT@"],
                shell: None,
            });
        }
        v.push(Backend { requires: &["maim"], program: "maim", args: &["@OUT@"], shell: None });
        v.push(Backend { requires: &["scrot"], program: "scrot", args: &["@OUT@"], shell: None });
        v.push(Backend {
            requires: &["import"],
            program: "import",
            args: &["-window", "root", "@OUT@"],
            shell: None,
        });
        v.push(Backend {
            requires: &["gnome-screenshot"],
            program: "gnome-screenshot",
            args: &["-f", "@OUT@"],
            shell: None,
        });
    }

    v
}

fn on_path(bin: &str) -> bool {
    if let Ok(paths) = std::env::var("PATH") {
        for dir in std::env::split_paths(&paths) {
            if dir.join(bin).is_file() {
                return true;
            }
        }
    }
    false
}

fn temp_png() -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("linkshot-capture-{}.png", std::process::id()));
    p
}

/// Grab the whole screen and return it as an RGBA image.
pub fn capture_fullscreen() -> Result<image::RgbaImage> {
    let out = temp_png();
    let _ = std::fs::remove_file(&out);
    let out_str = out.to_string_lossy().to_string();

    let mut tried: Vec<&str> = Vec::new();
    for b in fullscreen_backends() {
        if !b.requires.iter().all(|r| on_path(r)) {
            continue;
        }
        tried.push(b.requires[0]);

        let status = if let Some(sh) = b.shell {
            Command::new("sh").arg("-c").arg(sh.replace("@OUT@", &out_str)).status()
        } else {
            let args: Vec<String> = b.args.iter().map(|a| a.replace("@OUT@", &out_str)).collect();
            Command::new(b.program).args(&args).status()
        }
        .with_context(|| format!("failed to launch capture tool `{}`", b.requires[0]))?;

        if !status.success() || !out.exists() {
            continue;
        }
        let img = image::open(&out)
            .with_context(|| format!("failed to decode capture from {out_str}"))?
            .to_rgba8();
        let _ = std::fs::remove_file(&out);
        return Ok(img);
    }

    if tried.is_empty() {
        Err(anyhow!(
            "No screenshot tool found. Install one of: grim, gnome-screenshot, \
             spectacle, maim, scrot, or ImageMagick (import)."
        ))
    } else {
        Err(anyhow!("Full-screen capture failed using: {}", tried.join(", ")))
    }
}
