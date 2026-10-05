//! Where the token comes from, in precedence order:
//!
//! 1. `CLOUDFLARE_API_TOKEN` in the environment
//! 2. `~/.config/<app>/credentials.toml` written by this tool (mode 0600)
//! 3. the `cf` CLI's login (`<cf config dir>/config/default.json`), opt-in
//! 4. wrangler's login (`<wrangler config dir>/config/default.toml`), opt-in
//!
//! Tokens from `cf`/wrangler are OAuth access tokens with an expiry. We never
//! refresh them with their refresh token; when expired we tell the user to
//! run `cf auth login` / `wrangler login` again.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::api::{ApiResult, Client};

pub const APP_DIR: &str = "flaretop";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CredSource {
    Env,
    ConfigFile,
    CfCli,
    Wrangler,
    Prompt,
}

impl CredSource {
    pub fn label(self) -> &'static str {
        match self {
            CredSource::Env => "env",
            CredSource::ConfigFile => "config",
            CredSource::CfCli => "cf login",
            CredSource::Wrangler => "wrangler",
            CredSource::Prompt => "pasted",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Credential {
    pub token: String,
    pub source: CredSource,
    /// RFC 3339 expiry for OAuth tokens, if known.
    pub expires: Option<String>,
    pub path: Option<PathBuf>,
}

impl Credential {
    pub fn is_expired(&self) -> bool {
        match &self.expires {
            None => false,
            Some(ts) => {
                // Compare lexicographically on the RFC 3339 prefix; good enough
                // for "is it in the past" at second resolution in UTC.
                let now = now_rfc3339();
                ts.get(..19).unwrap_or(ts) < now.as_str()
            }
        }
    }
}

fn now_rfc3339() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as i64;
    civil_from_unix(secs)
}

/// "YYYY-MM-DDTHH:MM:SS" from a unix timestamp (UTC). Howard Hinnant's
/// days-to-civil algorithm, no chrono dependency.
pub fn civil_from_unix(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        y,
        m,
        d,
        rem / 3600,
        (rem / 60) % 60,
        rem % 60
    )
}

pub fn config_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join(APP_DIR))
}

/// One named connection: a token and, optionally, the account it opens.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Profile {
    pub api_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
}

/// `~/.config/flaretop/credentials.toml`. The top-level `api_token` is the
/// pre-profiles layout and is read as the profile named "default".
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct StoredCredentials {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_token: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub profiles: BTreeMap<String, Profile>,
}

pub const DEFAULT_PROFILE: &str = "default";

pub fn credentials_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("credentials.toml"))
}

fn read_stored() -> StoredCredentials {
    let Some(path) = credentials_path() else {
        return StoredCredentials::default();
    };
    let mut stored: StoredCredentials = fs::read_to_string(&path)
        .ok()
        .and_then(|t| toml::from_str(&t).ok())
        .unwrap_or_default();
    if let Some(t) = stored.api_token.take() {
        stored
            .profiles
            .entry(DEFAULT_PROFILE.to_string())
            .or_insert(Profile {
                api_token: t,
                account_id: None,
            });
    }
    stored
}

fn write_stored(stored: &StoredCredentials) -> std::io::Result<PathBuf> {
    let path = credentials_path()
        .ok_or_else(|| std::io::Error::other("no config directory on this platform"))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let body = toml::to_string(stored).map_err(std::io::Error::other)?;
    write_private(&path, body.as_bytes())?;
    Ok(path)
}

/// All saved profiles, sorted by name.
pub fn profiles() -> Vec<(String, Profile)> {
    read_stored().profiles.into_iter().collect()
}

pub fn load_profile(name: &str) -> Option<Credential> {
    let stored = read_stored();
    let p = stored.profiles.get(name)?;
    Some(Credential {
        token: p.api_token.clone(),
        source: CredSource::ConfigFile,
        expires: None,
        path: credentials_path(),
    })
}

/// The "default" profile, for callers that do not care about names.
pub fn load_stored() -> Option<Credential> {
    load_profile(DEFAULT_PROFILE)
}

pub fn save_profile(name: &str, token: &str, account_id: Option<&str>) -> std::io::Result<PathBuf> {
    let mut stored = read_stored();
    stored.profiles.insert(
        name.trim().to_string(),
        Profile {
            api_token: token.trim().to_string(),
            account_id: account_id.map(str::to_string),
        },
    );
    write_stored(&stored)
}

pub fn save_token(token: &str) -> std::io::Result<PathBuf> {
    save_profile(DEFAULT_PROFILE, token, None)
}

pub fn forget_profile(name: &str) -> std::io::Result<bool> {
    let mut stored = read_stored();
    let existed = stored.profiles.remove(name).is_some();
    if stored.profiles.is_empty() {
        if let Some(path) = credentials_path()
            && path.exists()
        {
            fs::remove_file(path)?;
        }
    } else {
        write_stored(&stored)?;
    }
    Ok(existed)
}

pub fn forget_token() -> std::io::Result<()> {
    forget_profile(DEFAULT_PROFILE).map(|_| ())
}

fn write_private(path: &PathBuf, bytes: &[u8]) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write as _;
        use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        f.write_all(bytes)?;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
        Ok(())
    }
    #[cfg(not(unix))]
    {
        fs::write(path, bytes)
    }
}

/// The `cf` CLI's credential file. macOS keeps it under
/// `~/Library/Preferences/cloudflare`, everything else under
/// `$XDG_CONFIG_HOME/cloudflare` (default `~/.config/cloudflare`).
pub fn cf_cli_paths() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if cfg!(target_os = "macos")
        && let Some(home) = dirs::home_dir()
    {
        out.push(
            home.join("Library")
                .join("Preferences")
                .join("cloudflare")
                .join("config")
                .join("default.json"),
        );
    }
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        out.push(
            PathBuf::from(xdg)
                .join("cloudflare")
                .join("config")
                .join("default.json"),
        );
    }
    if let Some(home) = dirs::home_dir() {
        out.push(
            home.join(".config")
                .join("cloudflare")
                .join("config")
                .join("default.json"),
        );
    }
    out
}

#[derive(Debug, Deserialize)]
struct CfCliFile {
    oauth_token: Option<String>,
    expiration_time: Option<String>,
    api_token: Option<String>,
}

pub fn load_cf_cli() -> Option<Credential> {
    for path in cf_cli_paths() {
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(f) = serde_json::from_str::<CfCliFile>(&text) else {
            continue;
        };
        if let Some(t) = f.api_token.filter(|t| !t.is_empty()) {
            return Some(Credential {
                token: t,
                source: CredSource::CfCli,
                expires: None,
                path: Some(path),
            });
        }
        if let Some(t) = f.oauth_token.filter(|t| !t.is_empty()) {
            return Some(Credential {
                token: t,
                source: CredSource::CfCli,
                expires: f.expiration_time,
                path: Some(path),
            });
        }
    }
    None
}

/// wrangler: legacy `~/.wrangler` if it exists, else the XDG config dir.
pub fn wrangler_paths() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(home) = dirs::home_dir() {
        out.push(home.join(".wrangler").join("config").join("default.toml"));
    }
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        out.push(
            PathBuf::from(xdg)
                .join(".wrangler")
                .join("config")
                .join("default.toml"),
        );
    }
    if let Some(home) = dirs::home_dir() {
        out.push(
            home.join(".config")
                .join(".wrangler")
                .join("config")
                .join("default.toml"),
        );
    }
    out
}

#[derive(Debug, Deserialize)]
struct WranglerFile {
    oauth_token: Option<String>,
    expiration_time: Option<String>,
    api_token: Option<String>,
}

pub fn load_wrangler() -> Option<Credential> {
    for path in wrangler_paths() {
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(f) = toml::from_str::<WranglerFile>(&text) else {
            continue;
        };
        if let Some(t) = f.api_token.filter(|t| !t.is_empty()) {
            return Some(Credential {
                token: t,
                source: CredSource::Wrangler,
                expires: None,
                path: Some(path),
            });
        }
        if let Some(t) = f.oauth_token.filter(|t| !t.is_empty()) {
            return Some(Credential {
                token: t,
                source: CredSource::Wrangler,
                expires: f.expiration_time,
                path: Some(path),
            });
        }
    }
    None
}

/// Every credential we can find, in precedence order. The caller decides
/// whether to use the CLI logins (they are opt-in by flag or prompt).
pub fn discover() -> Vec<Credential> {
    let mut out = Vec::new();
    if let Ok(t) = std::env::var("CLOUDFLARE_API_TOKEN")
        && !t.trim().is_empty()
    {
        out.push(Credential {
            token: t,
            source: CredSource::Env,
            expires: None,
            path: None,
        });
    }
    if let Some(c) = load_stored() {
        out.push(c);
    }
    if let Some(c) = load_cf_cli() {
        out.push(c);
    }
    if let Some(c) = load_wrangler() {
        out.push(c);
    }
    out
}

#[derive(Debug, Clone, Deserialize)]
pub struct TokenStatus {
    pub id: Option<String>,
    pub status: String,
    #[serde(default)]
    pub expires_on: Option<String>,
}

pub async fn verify(client: &Client) -> ApiResult<TokenStatus> {
    client.get("/user/tokens/verify", &[]).await
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Account {
    pub id: String,
    pub name: String,
}

pub async fn accounts(client: &Client) -> ApiResult<Vec<Account>> {
    client.get_all_pages("/accounts", 50, &[]).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_token_becomes_default_profile() {
        let stored: StoredCredentials = toml::from_str("api_token = \"abc\"\n").unwrap();
        assert_eq!(stored.api_token.as_deref(), Some("abc"));
        let text = toml::to_string(&StoredCredentials {
            api_token: None,
            profiles: BTreeMap::from([(
                "work".to_string(),
                Profile {
                    api_token: "t".into(),
                    account_id: Some("a".into()),
                },
            )]),
        })
        .unwrap();
        assert!(text.contains("[profiles.work]"), "{text}");
    }

    #[test]
    fn civil_dates() {
        assert_eq!(civil_from_unix(0), "1970-01-01T00:00:00");
        assert_eq!(civil_from_unix(1_700_000_000), "2023-11-14T22:13:20");
    }

    #[test]
    fn expiry_compares() {
        let c = Credential {
            token: "x".into(),
            source: CredSource::CfCli,
            expires: Some("2000-01-01T00:00:00.000Z".into()),
            path: None,
        };
        assert!(c.is_expired());
        let c = Credential {
            expires: Some("2999-01-01T00:00:00Z".into()),
            ..c
        };
        assert!(!c.is_expired());
    }
}
