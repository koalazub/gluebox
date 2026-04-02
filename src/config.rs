use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

fn default_sessions_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join("Library/Application Support/hyprnote/sessions")
}
fn default_hyprnote_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join("Library/Application Support/hyprnote")
}
fn default_debounce_secs() -> u64 {
    30
}
fn default_uni_calendar_names() -> Vec<String> {
    vec!["Uni".into()]
}
fn default_turso() -> TursoConfig {
    // Default to local embedded replica for resilience:
    // - Local-first operation (fast, works offline)
    // - Concurrency via turso's embedded replica architecture
    // - Easy migration path: set url/auth_token later to sync to remote Turso
    TursoConfig {
        url: String::new(),
        auth_token: String::new(),
        replica_path: Some(PathBuf::from("/var/lib/gluebox/turso")),
        sync_interval_secs: Some(60),
        encryption_key: None,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Config {
    pub listen_addr: String,
    pub notify_secret: Option<String>,
    pub linear: Option<LinearConfig>,
    pub anytype: Option<AnytypeConfig>,
    pub matrix: Option<MatrixConfig>,
    pub documenso: Option<DocumensoConfig>,
    pub opencode: Option<OpenCodeConfig>,
    #[serde(default = "default_turso")]
    pub turso: TursoConfig,
    pub github: Option<GithubConfig>,
    pub socket_path: Option<String>,
    pub power: Option<PowerConfig>,
    #[serde(default, deserialize_with = "deserialize_affine_workspaces")]
    pub affine: HashMap<String, AffineConfig>,
    pub watcher: Option<WatcherConfig>,
    pub stonkwatch_social: Option<StonkwatchSocialConfig>,
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

fn default_output_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join("Documents/gluebox-study")
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AffineConfig {
    pub api_url: String,
    pub api_token: String,
    pub workspace_id: String,
    /// Streamable-HTTP MCP endpoint for this workspace (e.g.
    /// https://app.affine.pro/api/workspaces/{id}/mcp).
    /// Used for doc creation when available.
    #[serde(default)]
    pub mcp_url: Option<String>,
    #[serde(default = "default_output_dir")]
    pub output_dir: PathBuf,
}

/// Deserialize `[affine]` as either a single workspace (old format) or
/// a map of named workspaces (`[affine.default]`, `[affine.stonkington]`).
fn deserialize_affine_workspaces<'de, D>(
    deserializer: D,
) -> Result<HashMap<String, AffineConfig>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum AffineEntry {
        Single(AffineConfig),
        Multi(HashMap<String, AffineConfig>),
    }
    match Option::<AffineEntry>::deserialize(deserializer)? {
        None => Ok(HashMap::new()),
        Some(AffineEntry::Single(cfg)) => {
            let mut map = HashMap::new();
            map.insert("default".into(), cfg);
            Ok(map)
        }
        Some(AffineEntry::Multi(map)) => Ok(map),
    }
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StonkwatchSocialConfig {
    pub turso_url: String,
    pub turso_auth_token: String,
    pub openrouter_api_key: Option<String>,
    pub post_interval_secs: Option<u64>,
    #[serde(default)]
    pub auto_post: bool,
    pub review_room_id: Option<String>,
    pub x: Option<XConfig>,
    pub bluesky: Option<BlueskyConfig>,
    pub meta: Option<MetaConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct XConfig {
    pub client_id: String,
    pub client_secret: String,
    pub access_token: String,
    pub refresh_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BlueskyConfig {
    pub identifier: String,
    pub password: String,
    #[serde(default = "default_bluesky_service_url")]
    pub service_url: String,
}

fn default_bluesky_service_url() -> String {
    "https://bsky.social".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MetaConfig {
    pub page_access_token: String,
    pub page_id: String,
    pub ig_user_id: Option<String>,
    pub threads_user_id: Option<String>,
    #[serde(default = "default_true")]
    pub facebook_enabled: bool,
    #[serde(default = "default_true")]
    pub instagram_enabled: bool,
    #[serde(default = "default_true")]
    pub threads_enabled: bool,
}

fn default_true() -> bool { true }
