use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub listen_addr: String,
    pub notify_secret: Option<String>,
    pub linear: LinearConfig,
    pub anytype: Option<AnytypeConfig>,
    pub matrix: MatrixConfig,
    pub documenso: DocumensoConfig,
    pub opencode: Option<OpenCodeConfig>,
    pub turso: TursoConfig,
    pub github: Option<GithubConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TursoConfig {
    pub url: String,
    pub auth_token: String,
    pub replica_path: Option<PathBuf>,
    pub sync_interval_secs: Option<u64>,
    pub encryption_key: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GithubConfig {
    pub token: String,
    pub repo: String,
    pub webhook_secret: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OpenCodeConfig {
    pub api_key: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LinearConfig {
    pub api_key: String,
    pub webhook_secret: String,
    pub team_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AnytypeConfig {
    pub api_url: String,
    pub api_key: String,
    pub space_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MatrixConfig {
    pub homeserver_url: String,
    pub access_token: String,
    pub room_id: String,
    pub feedback_room_id: Option<String>,
    pub issues_room_id: Option<String>,
    pub bot_username: Option<String>,
    pub bot_password: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DocumensoConfig {
    pub api_url: String,
    pub api_key: String,
    pub webhook_secret: String,
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
