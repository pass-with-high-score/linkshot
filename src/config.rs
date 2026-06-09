//! Configuration: where the Imgur Client-ID comes from.
//!
//! Resolution order:
//!   1. `IMGUR_CLIENT_ID` environment variable
//!   2. `~/.config/linkshot/config.json`  ->  { "imgur_client_id": "..." }

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Default)]
pub struct Config {
    pub imgur_client_id: Option<String>,
}

fn config_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("dev", "linkshot", "linkshot")
        .map(|d| d.config_dir().join("config.json"))
}

impl Config {
    pub fn load() -> Self {
        if let Some(p) = config_path() {
            if let Ok(s) = std::fs::read_to_string(&p) {
                if let Ok(c) = serde_json::from_str::<Config>(&s) {
                    return c;
                }
            }
        }
        Config::default()
    }

    pub fn save(&self) -> Result<()> {
        let p = config_path().context("cannot determine config directory")?;
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&p, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }

    /// Effective Client-ID, env var taking precedence over the config file.
    pub fn client_id(&self) -> Option<String> {
        if let Ok(v) = std::env::var("IMGUR_CLIENT_ID") {
            if !v.trim().is_empty() {
                return Some(v.trim().to_string());
            }
        }
        self.imgur_client_id
            .as_ref()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }
}
