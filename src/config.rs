use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

fn default_samaya_binary() -> String {
    "samaya".to_string()
}
fn default_calendar_name() -> String {
    "Uni".to_string()
}
fn default_match_keywords() -> Vec<String> {
    vec!["lecture".into(), "tutorial".into(), "seminar".into()]
}
fn default_samaya_output_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join("Documents/samaya-recordings")
}
fn default_pre_event_minutes() -> u64 {
    2
}
fn default_post_event_minutes() -> u64 {
    5
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
    pub stonkwatch_social: Option<StonkwatchSocialConfig>,
    pub samaya: Option<SamayaConfig>,
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
    pub error_rollup_room_id: Option<String>,
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
pub struct SamayaConfig {
    #[serde(default = "default_samaya_binary")]
    pub binary: String,
    #[serde(default = "default_calendar_name")]
    pub calendar_name: String,
    #[serde(default = "default_match_keywords")]
    pub match_keywords: Vec<String>,
    #[serde(default = "default_samaya_output_dir")]
    pub output_dir: PathBuf,
    #[serde(default = "default_pre_event_minutes")]
    pub pre_event_minutes: u64,
    #[serde(default = "default_post_event_minutes")]
    pub post_event_minutes: u64,
    pub note_extraction_prompt: Option<String>,
    pub affine_workspace: Option<String>,
}

impl Default for SamayaConfig {
    fn default() -> Self {
        Self {
            binary: default_samaya_binary(),
            calendar_name: default_calendar_name(),
            match_keywords: default_match_keywords(),
            output_dir: default_samaya_output_dir(),
            pre_event_minutes: default_pre_event_minutes(),
            post_event_minutes: default_post_event_minutes(),
            note_extraction_prompt: None,
            affine_workspace: None,
        }
    }
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
    #[serde(default = "default_max_posts_per_cycle")]
    pub max_posts_per_cycle: u32,
    #[serde(default = "default_trending_only")]
    pub trending_only: bool,
    #[serde(default = "default_trending_webhook_driven")]
    pub trending_webhook_driven: bool,
    pub x: Option<XConfig>,
    pub bluesky: Option<BlueskyConfig>,
    pub meta: Option<MetaConfig>,
    pub storj: Option<StorjConfig>,
    pub tiktok: Option<TikTokConfig>,
    pub chart_video_api_base: Option<String>,
    pub stonkwatch_api_key: Option<String>,
    pub friday_digest_enabled: Option<bool>,
}

fn default_max_posts_per_cycle() -> u32 { 1 }
fn default_trending_only() -> bool { true }
fn default_trending_webhook_driven() -> bool { true }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StorjConfig {
    pub access_key: String,
    pub secret_key: String,
    pub bucket: String,
    #[serde(default = "default_storj_endpoint")]
    pub endpoint: String,
    #[serde(default = "default_storj_public_base")]
    pub public_base_url: String,
}

fn default_storj_endpoint() -> String {
    "https://gateway.storjshare.io".to_string()
}

fn default_storj_public_base() -> String {
    "https://link.storjshare.io/raw".to_string()
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
    pub threads_access_token: Option<String>,
    #[serde(default = "default_true")]
    pub facebook_enabled: bool,
    #[serde(default = "default_true")]
    pub instagram_enabled: bool,
    #[serde(default = "default_true")]
    pub threads_enabled: bool,
}

fn default_true() -> bool { true }

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct TikTokConfig {
    pub client_key: String,
    pub client_secret: String,
    pub access_token: String,
    pub refresh_token: Option<String>,
    #[serde(default = "default_tiktok_privacy_level")]
    pub privacy_level: String,
    #[serde(default)]
    pub disable_duet: bool,
    #[serde(default)]
    pub disable_stitch: bool,
    #[serde(default)]
    pub disable_comment: bool,
}

fn default_tiktok_privacy_level() -> String {
    "SELF_ONLY".to_string()
}
