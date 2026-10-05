//! `~/.config/flaretop/config.toml`: theme, last account, opt-ins.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Config {
    pub theme: Option<String>,
    pub account_id: Option<String>,
    /// Last used named profile from credentials.toml.
    pub profile: Option<String>,
    /// Reuse `cf auth login` / `wrangler login` tokens without asking.
    #[serde(default)]
    pub use_cli_logins: bool,
    /// Pane sizes, saved when changed with [ ] { } or by dragging a divider.
    pub sidebar_width: Option<u16>,
    pub list_percent: Option<u16>,
}

pub fn path() -> Option<PathBuf> {
    flaretop_core::auth::config_dir().map(|d| d.join("config.toml"))
}

pub fn load() -> Config {
    let Some(p) = path() else {
        return Config::default();
    };
    fs::read_to_string(p)
        .ok()
        .and_then(|t| toml::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn save(cfg: &Config) -> std::io::Result<()> {
    let Some(p) = path() else {
        return Ok(());
    };
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = toml::to_string(cfg).map_err(std::io::Error::other)?;
    fs::write(p, text)
}
