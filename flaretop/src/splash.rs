//! The start-up wordmark: the logo resolves out of noise in the scrollback
//! while the token is verified, then the TUI takes over. Skipped when stdout
//! is not a terminal, with `--no-anim`, `FLARETOP_NO_ANIM`, `NO_COLOR` or
//! `TERM=dumb`.

use std::io::{IsTerminal, Write, stdout};
use std::time::Duration;

use crossterm::ExecutableCommand;
use crossterm::cursor::{Hide, MoveUp, Show};

pub const LOGO: [&str; 6] = [
    "███████╗██╗      █████╗ ██████╗ ███████╗████████╗ ██████╗ ██████╗ ",
    "██╔════╝██║     ██╔══██╗██╔══██╗██╔════╝╚══██╔══╝██╔═══██╗██╔══██╗",
    "█████╗  ██║     ███████║██████╔╝█████╗     ██║   ██║   ██║██████╔╝",
    "██╔══╝  ██║     ██╔══██║██╔══██╗██╔══╝     ██║   ██║   ██║██╔═══╝ ",
    "██║     ███████╗██║  ██║██║  ██║███████╗   ██║   ╚██████╔╝██║     ",
    "╚═╝     ╚══════╝╚═╝  ╚═╝╚═╝  ╚═╝╚══════╝   ╚═╝    ╚═════╝ ╚═╝     ",
];
pub const TAGLINE: &str = "top for Cloudflare";
const VERSION: &str = env!("CARGO_PKG_VERSION");
const ORANGE: &str = "\x1b[38;2;246;130;31m";
const AMBER: &str = "\x1b[38;2;250;173;63m";
const DIM: &str = "\x1b[2m";
const BOLD: &str = "\x1b[1m";
const RESET: &str = "\x1b[0m";

pub fn enabled(no_anim_flag: bool) -> bool {
    !no_anim_flag
        && stdout().is_terminal()
        && std::env::var_os("NO_COLOR").is_none()
        && std::env::var_os("FLARETOP_NO_ANIM").is_none()
        && std::env::var("TERM").map(|t| t != "dumb").unwrap_or(true)
}

fn hash(x: usize, y: usize) -> f64 {
    let mut h = (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (y as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 31;
    h = h.wrapping_mul(0x94D0_49BB_1331_11EB);
    h ^= h >> 29;
    (h % 10_000) as f64 / 10_000.0
}

/// One frame: a left-to-right sweep with per-cell jitter, every cell
/// passing through ░ ▒ ▓ before it settles. `t` in 0..1.
fn frame(t: f64) -> Vec<String> {
    let width = LOGO[0].chars().count();
    LOGO.iter()
        .enumerate()
        .map(|(y, row)| {
            let mut line = String::new();
            for (x, ch) in row.chars().enumerate() {
                if ch == ' ' {
                    line.push(' ');
                    continue;
                }
                let start = 0.55 * x as f64 / width as f64 + 0.3 * hash(x, y);
                let p = ((t - start) / 0.15).clamp(0.0, 1.0);
                if p <= 0.0 {
                    line.push(' ');
                } else if p < 0.35 {
                    line.push_str(&format!("{AMBER}░{RESET}"));
                } else if p < 0.7 {
                    line.push_str(&format!("{AMBER}▒{RESET}"));
                } else if p < 1.0 {
                    line.push_str(&format!("{ORANGE}▓{RESET}"));
                } else {
                    line.push_str(&format!("{ORANGE}{BOLD}{ch}{RESET}"));
                }
            }
            line
        })
        .collect()
}

/// Play the reveal in the scrollback (about 0.6 s) and leave the wordmark
/// plus one status line behind. Returns the number of lines written so the
/// caller can update the status line later with [`status`].
pub fn intro(what: &str) -> std::io::Result<()> {
    let mut out = stdout();
    let width = crossterm::terminal::size().map(|(w, _)| w).unwrap_or(80) as usize;
    let logo_w = LOGO[0].chars().count();
    if width < logo_w + 2 {
        writeln!(out, "{ORANGE}▲{RESET} flaretop v{VERSION} · {what}")?;
        return Ok(());
    }
    out.execute(Hide)?;
    let frames = 16;
    for i in 0..=frames {
        let t = i as f64 / frames as f64;
        if i > 0 {
            out.execute(MoveUp(LOGO.len() as u16))?;
        }
        for line in frame(t) {
            writeln!(out, "\x1b[2K{line}")?;
        }
        out.flush()?;
        std::thread::sleep(Duration::from_millis(38));
    }
    writeln!(out, "{DIM}{TAGLINE} · v{VERSION} · {what}{RESET}")?;
    out.flush()?;
    out.execute(Show)?;
    Ok(())
}

/// Replace the status line left by [`intro`].
pub fn status(what: &str) {
    let mut out = stdout();
    let _ = out.execute(MoveUp(1));
    let _ = writeln!(out, "\x1b[2K{DIM}{TAGLINE} · v{VERSION} · {what}{RESET}");
    let _ = out.flush();
}
