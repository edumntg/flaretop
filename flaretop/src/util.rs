//! Clipboard, clock and text helpers.

use std::io::Write as _;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use unicode_width::UnicodeWidthStr;

/// Copy text to the system clipboard by shelling out (no heavy deps).
pub fn clipboard_copy(text: &str) -> Result<(), String> {
    let candidates: &[(&str, &[&str])] = if cfg!(target_os = "macos") {
        &[("pbcopy", &[])]
    } else if cfg!(target_os = "windows") {
        &[("clip", &[])]
    } else {
        &[
            ("wl-copy", &[]),
            ("xclip", &["-selection", "clipboard"]),
            ("xsel", &["--clipboard", "--input"]),
        ]
    };
    for (bin, args) in candidates {
        let child = Command::new(bin)
            .args(*args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        if let Ok(mut child) = child {
            if let Some(stdin) = child.stdin.as_mut()
                && stdin.write_all(text.as_bytes()).is_ok()
            {
                let _ = child.wait();
                return Ok(());
            }
            let _ = child.wait();
        }
    }
    // OSC 52 as a last resort: works over SSH in terminals that allow it.
    let b64 = base64(text.as_bytes());
    let mut out = std::io::stdout();
    if out
        .write_all(format!("\x1b]52;c;{b64}\x07").as_bytes())
        .is_ok()
        && out.flush().is_ok()
    {
        return Ok(());
    }
    Err("no clipboard tool found (pbcopy/wl-copy/xclip/xsel/clip)".into())
}

fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

fn timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Session clock "hh:mm:ss" (UTC) for the ops log.
pub fn clock_utc() -> String {
    let s = timestamp();
    format!("{:02}:{:02}:{:02}", (s / 3600) % 24, (s / 60) % 60, s % 60)
}

/// "hh:mm:ss" from unix milliseconds.
pub fn clock_from_ms(ms: u64) -> String {
    let s = ms / 1000;
    format!("{:02}:{:02}:{:02}", (s / 3600) % 24, (s / 60) % 60, s % 60)
}

/// Clip to `width` display cells, appending `…` when cut.
pub fn fit(s: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    if UnicodeWidthStr::width(s) <= width {
        return s.to_string();
    }
    let mut out = String::new();
    let mut w = 0;
    for ch in s.chars() {
        let cw = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if w + cw > width.saturating_sub(1) {
            break;
        }
        out.push(ch);
        w += cw;
    }
    out.push('…');
    out
}

/// Pad or clip to exactly `width` cells.
pub fn pad(s: &str, width: usize) -> String {
    let f = fit(s, width);
    let w = UnicodeWidthStr::width(f.as_str());
    format!("{f}{}", " ".repeat(width.saturating_sub(w)))
}

/// One-line, control-char-free preview of a value.
pub fn one_line(s: &str, width: usize) -> String {
    let collapsed: String = s
        .chars()
        .map(|c| {
            if c == '\n' || c == '\r' || c == '\t' {
                ' '
            } else {
                c
            }
        })
        .filter(|c| !c.is_control())
        .collect();
    fit(collapsed.trim(), width)
}

/// Pretty-print bytes as text when they look like text (JSON pretty when
/// they parse as JSON); otherwise a hex dump of the head.
pub fn bytes_to_text(bytes: &[u8]) -> (String, bool) {
    if let Ok(s) = std::str::from_utf8(bytes) {
        let printable = s
            .chars()
            .filter(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
            .count();
        if printable == 0 || !s.is_empty() && printable * 50 < s.len() {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(s)
                && (v.is_object() || v.is_array())
            {
                return (
                    serde_json::to_string_pretty(&v).unwrap_or_else(|_| s.to_string()),
                    true,
                );
            }
            return (s.to_string(), true);
        }
    }
    let mut out = String::new();
    for (i, chunk) in bytes.chunks(16).take(256).enumerate() {
        let hex: Vec<String> = chunk.iter().map(|b| format!("{b:02x}")).collect();
        let ascii: String = chunk
            .iter()
            .map(|&b| {
                if (0x20..0x7f).contains(&b) {
                    b as char
                } else {
                    '.'
                }
            })
            .collect();
        out.push_str(&format!(
            "{:08x}  {:<48}  {}\n",
            i * 16,
            hex.join(" "),
            ascii
        ));
    }
    if bytes.len() > 16 * 256 {
        out.push_str(&format!("… {} more bytes\n", bytes.len() - 16 * 256));
    }
    (out, false)
}

pub fn guess_content_type(path: &str) -> &'static str {
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "json" => "application/json",
        "txt" | "md" | "csv" | "log" => "text/plain",
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "js" | "mjs" => "application/javascript",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        "wasm" => "application/wasm",
        "mp4" => "video/mp4",
        "mp3" => "audio/mpeg",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_and_pad() {
        assert_eq!(fit("hello", 10), "hello");
        assert_eq!(fit("hello world", 6), "hello…");
        assert_eq!(pad("ab", 4), "ab  ");
    }

    #[test]
    fn json_bytes_pretty() {
        let (t, text) = bytes_to_text(b"{\"a\":1}");
        assert!(text);
        assert!(t.contains("\n"));
        let (_, text) = bytes_to_text(&[0, 1, 2, 255, 254]);
        assert!(!text);
    }

    #[test]
    fn b64() {
        assert_eq!(base64(b"hi"), "aGk=");
    }
}
