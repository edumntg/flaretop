//! Core of flaretop: the Cloudflare API client, authentication, product
//! modules and the I/O actor. No TUI dependency, so everything here is
//! testable against recorded JSON.

pub mod actor;
pub mod api;
pub mod auth;
pub mod products;
pub mod tail;
pub mod types;

/// Install the ring crypto provider. reqwest 0.13 ships no provider when
/// built with `rustls-no-provider`; this must run once before the first TLS
/// connection.
pub fn init_crypto() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// Human-friendly byte count.
pub fn human_bytes(n: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} B")
    } else if v >= 100.0 {
        format!("{v:.0} {}", UNITS[i])
    } else {
        format!("{v:.1} {}", UNITS[i])
    }
}

/// "2026-10-05T12:34:56.789Z" -> "2026-10-05 12:34"
pub fn short_time(ts: &str) -> String {
    let t = ts.replace('T', " ");
    t.chars().take(16).collect()
}

/// Relative age like "3m", "2h", "5d" from an RFC 3339 timestamp.
pub fn age(ts: &str) -> String {
    let Some(secs) = parse_rfc3339_secs(ts) else {
        return String::new();
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as i64;
    let d = (now - secs).max(0);
    if d < 60 {
        format!("{d}s")
    } else if d < 3600 {
        format!("{}m", d / 60)
    } else if d < 86_400 {
        format!("{}h", d / 3600)
    } else if d < 86_400 * 365 {
        format!("{}d", d / 86_400)
    } else {
        format!("{}y", d / (86_400 * 365))
    }
}

/// Parse "YYYY-MM-DDTHH:MM:SS[.fff]Z" (UTC only) to unix seconds.
pub fn parse_rfc3339_secs(ts: &str) -> Option<i64> {
    let b = ts.as_bytes();
    if b.len() < 19 {
        return None;
    }
    let num = |s: &str| s.parse::<i64>().ok();
    let y = num(&ts[0..4])?;
    let m = num(&ts[5..7])?;
    let d = num(&ts[8..10])?;
    let hh = num(&ts[11..13])?;
    let mm = num(&ts[14..16])?;
    let ss = num(&ts[17..19])?;
    // days from civil (Howard Hinnant)
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + hh * 3600 + mm * 60 + ss)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_format() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(2048), "2.0 KB");
        assert_eq!(human_bytes(221_184), "216 KB");
    }

    #[test]
    fn rfc3339_roundtrip() {
        assert_eq!(
            parse_rfc3339_secs("2023-11-14T22:13:20.000Z"),
            Some(1_700_000_000)
        );
        assert_eq!(auth::civil_from_unix(1_700_000_000), "2023-11-14T22:13:20");
    }
}
