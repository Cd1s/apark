use serde::{Deserialize, Serialize};

use crate::paths;

/// User settings, stored in `<home>/config.json`.
///
/// OAuth client credentials resolve in this order: environment variable,
/// config file, value baked in at build time (CI release builds).
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Config {
    pub google_client_id: Option<String>,
    pub google_client_secret: Option<String>,
    pub microsoft_client_id: Option<String>,
    /// Where the account list is synced (Google Drive, self-hosted server, WebDAV, file).
    pub sync: Option<crate::cloud::SyncTarget>,
    /// Encrypts the synced account list (required for WebDAV and file targets).
    pub sync_passphrase: Option<String>,
    pub sync_interval_secs: u64,
    /// How many recent messages per folder the first sync downloads.
    pub initial_limit: u32,
    /// Bodies up to this size are prefetched for offline reading.
    pub prefetch_kb: u32,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            google_client_id: None,
            google_client_secret: None,
            microsoft_client_id: None,
            sync: None,
            sync_passphrase: None,
            sync_interval_secs: 120,
            initial_limit: 500,
            prefetch_kb: 512,
        }
    }
}

/// First non-empty of: environment variable, config file, build-time value.
fn pick(env: &str, file: &Option<String>, baked: Option<&'static str>) -> Option<String> {
    let set = |s: &String| !s.trim().is_empty();
    std::env::var(env)
        .ok()
        .filter(set)
        .or_else(|| file.clone().filter(set))
        .or_else(|| baked.map(str::to_owned).filter(set))
}

impl Config {
    pub fn load() -> Config {
        let path = paths::home().join("config.json");
        std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let dir = paths::ensure_home()?;
        paths::write_private(&dir.join("config.json"), &serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }

    pub fn google_client(&self) -> Option<(String, Option<String>)> {
        let id = pick("APARK_GOOGLE_CLIENT_ID", &self.google_client_id, option_env!("APARK_GOOGLE_CLIENT_ID"))?;
        let secret = pick(
            "APARK_GOOGLE_CLIENT_SECRET",
            &self.google_client_secret,
            option_env!("APARK_GOOGLE_CLIENT_SECRET"),
        );
        Some((id, secret))
    }

    pub fn microsoft_client(&self) -> Option<String> {
        pick("APARK_MS_CLIENT_ID", &self.microsoft_client_id, option_env!("APARK_MS_CLIENT_ID"))
    }

    pub fn passphrase(&self) -> Option<String> {
        pick("APARK_SYNC_PASSPHRASE", &self.sync_passphrase, None)
    }
}
