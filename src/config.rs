use serde::{Deserialize, Serialize};
use std::path::PathBuf;

fn default_sessions_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join("Library/Application Support/hyprnote/sessions")
}
fn default_hyprnote_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join("Library/Application Support/hyprnote")
}
fn default_debounce_secs() -> u64 { 30 }
fn default_uni_calendar_names() -> Vec<String> { vec!["Uni".into()] }

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
    pub affine: Option<AffineConfig>,
    pub watcher: Option<WatcherConfig>,
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
pub struct AffineConfig {
    pub api_url: String,
    pub api_token: String,
    pub workspace_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WatcherConfig {
    #[serde(default = "default_sessions_dir")]
    pub sessions_dir: PathBuf,
    #[serde(default = "default_hyprnote_dir")]
    pub hyprnote_dir: PathBuf,
    #[serde(default = "default_debounce_secs")]
    pub debounce_secs: u64,
    #[serde(default = "default_uni_calendar_names")]
    pub uni_calendar_names: Vec<String>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_partial_eq_detects_change() {
        let a = LinearConfig {
            api_key: "key-a".into(),
            webhook_secret: "secret".into(),
            team_id: None,
        };
        let b = LinearConfig {
            api_key: "key-b".into(),
            webhook_secret: "secret".into(),
            team_id: None,
        };
        assert_ne!(a, b);
    }

    #[test]
    fn config_partial_eq_same_values() {
        let a = LinearConfig {
            api_key: "key-same".into(),
            webhook_secret: "secret-same".into(),
            team_id: Some("team-1".into()),
        };
        let b = LinearConfig {
            api_key: "key-same".into(),
            webhook_secret: "secret-same".into(),
            team_id: Some("team-1".into()),
        };
        assert_eq!(a, b);
    }

    #[test]
    fn power_config_default_values() {
        let p = PowerConfig::default();
        assert!((p.threshold - 5.0).abs() < f64::EPSILON);
        assert!((p.decay_rate - 0.5).abs() < f64::EPSILON);
        assert_eq!(p.tick_interval_secs, 30);
        assert!((p.spike_weight - 2.0).abs() < f64::EPSILON);
        assert_eq!(p.min_active_secs, 10);
    }
}
