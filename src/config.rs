use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Config {
    pub listen_addr: String,
    pub notify_secret: Option<String>,
    pub linear: Option<LinearConfig>,
    pub anytype: Option<AnytypeConfig>,
    pub matrix: Option<MatrixConfig>,
    pub documenso: Option<DocumensoConfig>,
    pub opencode: Option<OpenCodeConfig>,
    pub turso: TursoConfig,
    pub github: Option<GithubConfig>,
    pub socket_path: Option<String>,
    pub power: Option<PowerConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TursoConfig {
    pub url: String,
    pub auth_token: String,
    pub replica_path: Option<PathBuf>,
    pub sync_interval_secs: Option<u64>,
    pub encryption_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GithubConfig {
    pub token: String,
    pub repo: String,
    pub webhook_secret: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OpenCodeConfig {
    pub api_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LinearConfig {
    pub api_key: String,
    pub webhook_secret: String,
    pub team_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AnytypeConfig {
    pub api_url: String,
    pub api_key: String,
    pub space_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MatrixConfig {
    pub homeserver_url: String,
    pub access_token: String,
    pub room_id: String,
    pub feedback_room_id: Option<String>,
    pub issues_room_id: Option<String>,
    pub bot_username: Option<String>,
    pub bot_password: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DocumensoConfig {
    pub api_url: String,
    pub api_key: String,
    pub webhook_secret: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PowerConfig {
    pub threshold: f64,
    pub decay_rate: f64,
    pub tick_interval_secs: u64,
    pub spike_weight: f64,
    pub min_active_secs: u64,
}

impl Default for PowerConfig {
    fn default() -> Self {
        Self {
            threshold: 5.0,
            decay_rate: 0.5,
            tick_interval_secs: 30,
            spike_weight: 2.0,
            min_active_secs: 10,
        }
    }
}

impl Config {
    pub fn load() -> anyhow::Result<Self> {
        let path = std::env::var("GLUEBOX_CONFIG").unwrap_or_else(|_| "gluebox.toml".to_string());
        let content = std::fs::read_to_string(&path)
            .map_err(|e| anyhow::anyhow!("failed to read config at {path}: {e}"))?;
        let cfg: Config = toml::from_str(&content)?;
        Ok(cfg)
    }
}
