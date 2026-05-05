use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use serde::Serialize;
use tokio::sync::{Mutex, RwLock};

use crate::triggers::error_rollup::ErrorRollup;

const STALE_THRESHOLD_SECS: i64 = 24 * 3600;
const WATCHDOG_INTERVAL_SECS: u64 = 1800;

pub struct PostingHeartbeat {
    last_webhook_received_at: AtomicI64,
    last_candidate_seen_at: AtomicI64,
    last_publish_success: Mutex<HashMap<String, i64>>,
    expected_platforms: RwLock<Vec<String>>,
}

#[derive(Serialize)]
pub struct PlatformHeartbeat {
    pub platform: String,
    pub last_success_at: i64,
    pub age_secs: i64,
    pub healthy: bool,
}

#[derive(Serialize)]
pub struct HeartbeatSnapshot {
    pub now: i64,
    pub last_webhook_received_at: Option<i64>,
    pub last_webhook_received_age_secs: Option<i64>,
    pub last_candidate_seen_at: Option<i64>,
    pub last_candidate_seen_age_secs: Option<i64>,
    pub stale_threshold_secs: i64,
    pub platforms: Vec<PlatformHeartbeat>,
}

impl PostingHeartbeat {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            last_webhook_received_at: AtomicI64::new(0),
            last_candidate_seen_at: AtomicI64::new(0),
            last_publish_success: Mutex::new(HashMap::new()),
            expected_platforms: RwLock::new(Vec::new()),
        })
    }

    pub fn record_webhook_received(&self) {
        self.last_webhook_received_at.store(now_secs(), Ordering::Relaxed);
    }

    pub fn record_candidate_seen(&self) {
        self.last_candidate_seen_at.store(now_secs(), Ordering::Relaxed);
    }

    pub async fn record_publish_success(&self, platform: &str) {
        self.last_publish_success.lock().await.insert(platform.to_string(), now_secs());
    }

    pub async fn set_expected_platforms(&self, platforms: Vec<String>) {
        let mut current = self.expected_platforms.write().await;
        if *current != platforms {
            tracing::info!(?platforms, "posting_heartbeat: expected_platforms updated");
            *current = platforms;
        }
    }

    pub async fn expected_platforms(&self) -> Vec<String> {
        self.expected_platforms.read().await.clone()
    }

    pub async fn snapshot(&self) -> HeartbeatSnapshot {
        let now = now_secs();
        let webhook = zero_to_none(self.last_webhook_received_at.load(Ordering::Relaxed));
        let candidate = zero_to_none(self.last_candidate_seen_at.load(Ordering::Relaxed));
        let map = self.last_publish_success.lock().await;
        let mut platforms: Vec<PlatformHeartbeat> = map
            .iter()
            .map(|(name, ts)| {
                let age = now - *ts;
                PlatformHeartbeat {
                    platform: name.clone(),
                    last_success_at: *ts,
                    age_secs: age,
                    healthy: age < STALE_THRESHOLD_SECS,
                }
            })
            .collect();
        platforms.sort_by(|a, b| a.platform.cmp(&b.platform));
        HeartbeatSnapshot {
            now,
            last_webhook_received_at: webhook,
            last_webhook_received_age_secs: webhook.map(|t| now - t),
            last_candidate_seen_at: candidate,
            last_candidate_seen_age_secs: candidate.map(|t| now - t),
            stale_threshold_secs: STALE_THRESHOLD_SECS,
            platforms,
        }
    }

    pub async fn snapshot_with_expected(
        &self,
        expected_platforms: &[String],
    ) -> HeartbeatSnapshot {
        let mut snap = self.snapshot().await;
        let now = snap.now;
        let known: std::collections::HashSet<&str> =
            snap.platforms.iter().map(|p| p.platform.as_str()).collect();
        let missing: Vec<PlatformHeartbeat> = expected_platforms
            .iter()
            .filter(|p| !known.contains(p.as_str()))
            .map(|p| PlatformHeartbeat {
                platform: p.clone(),
                last_success_at: 0,
                age_secs: now,
                healthy: false,
            })
            .collect();
        snap.platforms.extend(missing);
        snap.platforms.sort_by(|a, b| a.platform.cmp(&b.platform));
        snap
    }
}

pub fn spawn_watchdog(heartbeat: Arc<PostingHeartbeat>, error_rollup: Arc<ErrorRollup>) {
    tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(std::time::Duration::from_secs(WATCHDOG_INTERVAL_SECS));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        interval.tick().await;
        loop {
            interval.tick().await;
            let expected = heartbeat.expected_platforms().await;
            if expected.is_empty() {
                continue;
            }
            let snap = heartbeat.snapshot_with_expected(&expected).await;
            let expected_set: std::collections::HashSet<&str> =
                expected.iter().map(String::as_str).collect();
            let webhook_phrase = age_phrase(snap.last_webhook_received_age_secs);
            let candidate_phrase = age_phrase(snap.last_candidate_seen_age_secs);
            for p in &snap.platforms {
                if !expected_set.contains(p.platform.as_str()) {
                    continue;
                }
                if !p.healthy {
                    let success_phrase = if p.last_success_at == 0 {
                        "never".to_string()
                    } else {
                        format!("{}h ago", p.age_secs / 3600)
                    };
                    error_rollup
                        .record(
                            "platform_silence_detected",
                            format!(
                                "{}: last success {}, last webhook {}, last candidate {}",
                                p.platform, success_phrase, webhook_phrase, candidate_phrase,
                            ),
                        )
                        .await;
                }
            }
        }
    });
}

pub fn expected_platforms_from_config(
    cfg: &crate::config::StonkwatchSocialConfig,
) -> Vec<String> {
    if !cfg.auto_post {
        return Vec::new();
    }
    let mut out = Vec::new();
    if cfg.x.is_some() {
        out.push("x".to_string());
    }
    if cfg.bluesky.is_some() {
        out.push("bluesky".to_string());
    }
    if let Some(meta) = cfg.meta.as_ref() {
        if meta.facebook_enabled {
            out.push("facebook".to_string());
        }
        if meta.instagram_enabled && meta.ig_user_id.is_some() {
            out.push("instagram".to_string());
            out.push("instagram_story".to_string());
        }
        if meta.threads_enabled && meta.threads_user_id.is_some() {
            out.push("threads".to_string());
        }
    }
    if cfg.tiktok.is_some() && cfg.chart_video_api_base.is_some() {
        out.push("tiktok".to_string());
    }
    out
}

fn now_secs() -> i64 {
    chrono::Utc::now().timestamp()
}

fn zero_to_none(v: i64) -> Option<i64> {
    if v == 0 { None } else { Some(v) }
}

fn age_phrase(age: Option<i64>) -> String {
    match age {
        Some(s) if s < 3600 => format!("{}m ago", s / 60),
        Some(s) => format!("{}h ago", s / 3600),
        None => "never".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fresh_heartbeat_reports_no_activity() {
        let hb = PostingHeartbeat::new();
        let snap = hb.snapshot().await;
        assert_eq!(snap.last_webhook_received_at, None);
        assert_eq!(snap.last_candidate_seen_at, None);
        assert!(snap.platforms.is_empty());
        assert_eq!(snap.stale_threshold_secs, STALE_THRESHOLD_SECS);
    }

    #[tokio::test]
    async fn record_webhook_received_surfaces_in_snapshot() {
        let hb = PostingHeartbeat::new();
        hb.record_webhook_received();
        let snap = hb.snapshot().await;
        assert!(snap.last_webhook_received_at.is_some());
        let age = snap.last_webhook_received_age_secs.unwrap();
        assert!(age >= 0 && age < 5, "age should be ~0 immediately after record, got {age}");
    }

    #[tokio::test]
    async fn record_candidate_seen_surfaces_in_snapshot() {
        let hb = PostingHeartbeat::new();
        hb.record_candidate_seen();
        let snap = hb.snapshot().await;
        assert!(snap.last_candidate_seen_at.is_some());
        assert!(snap.last_candidate_seen_age_secs.unwrap() < 5);
    }

    #[tokio::test]
    async fn record_publish_success_marks_platform_healthy() {
        let hb = PostingHeartbeat::new();
        hb.record_publish_success("x").await;
        let snap = hb.snapshot().await;
        assert_eq!(snap.platforms.len(), 1);
        let p = &snap.platforms[0];
        assert_eq!(p.platform, "x");
        assert!(p.healthy);
        assert!(p.age_secs < 5);
    }

    #[tokio::test]
    async fn record_publish_success_overwrites_per_platform() {
        let hb = PostingHeartbeat::new();
        hb.record_publish_success("x").await;
        hb.record_publish_success("bluesky").await;
        hb.record_publish_success("x").await;
        let snap = hb.snapshot().await;
        assert_eq!(snap.platforms.len(), 2);
        let names: Vec<&str> = snap.platforms.iter().map(|p| p.platform.as_str()).collect();
        assert_eq!(names, vec!["bluesky", "x"]);
    }

    #[tokio::test]
    async fn snapshot_with_expected_marks_unrecorded_platforms_unhealthy() {
        let hb = PostingHeartbeat::new();
        hb.record_publish_success("x").await;
        let expected = vec!["x".to_string(), "bluesky".to_string(), "instagram".to_string()];
        let snap = hb.snapshot_with_expected(&expected).await;

        assert_eq!(snap.platforms.len(), 3);
        let bluesky = snap.platforms.iter().find(|p| p.platform == "bluesky").unwrap();
        assert!(!bluesky.healthy);
        assert_eq!(bluesky.last_success_at, 0);
        let instagram = snap.platforms.iter().find(|p| p.platform == "instagram").unwrap();
        assert!(!instagram.healthy);
        let x = snap.platforms.iter().find(|p| p.platform == "x").unwrap();
        assert!(x.healthy);
    }

    #[tokio::test]
    async fn snapshot_with_expected_keeps_unrelated_recorded_platforms() {
        let hb = PostingHeartbeat::new();
        hb.record_publish_success("tiktok").await;
        let expected = vec!["x".to_string()];
        let snap = hb.snapshot_with_expected(&expected).await;
        assert_eq!(snap.platforms.len(), 2);
        let names: Vec<&str> = snap.platforms.iter().map(|p| p.platform.as_str()).collect();
        assert_eq!(names, vec!["tiktok", "x"]);
    }

    #[tokio::test]
    async fn set_expected_platforms_replaces_previous_list() {
        let hb = PostingHeartbeat::new();
        hb.set_expected_platforms(vec!["x".into(), "bluesky".into()]).await;
        assert_eq!(hb.expected_platforms().await, vec!["x", "bluesky"]);

        hb.set_expected_platforms(vec!["instagram".into()]).await;
        assert_eq!(
            hb.expected_platforms().await,
            vec!["instagram"],
            "reload must replace, not merge — removed platforms must stop being expected"
        );
    }

    #[tokio::test]
    async fn set_expected_platforms_clears_when_config_removes_social() {
        let hb = PostingHeartbeat::new();
        hb.set_expected_platforms(vec!["x".into()]).await;
        hb.set_expected_platforms(Vec::new()).await;
        assert!(hb.expected_platforms().await.is_empty());
    }

    #[test]
    fn age_phrase_formats_minutes_under_one_hour() {
        assert_eq!(age_phrase(Some(0)), "0m ago");
        assert_eq!(age_phrase(Some(59)), "0m ago");
        assert_eq!(age_phrase(Some(120)), "2m ago");
        assert_eq!(age_phrase(Some(3599)), "59m ago");
    }

    #[test]
    fn age_phrase_formats_hours_above_one_hour() {
        assert_eq!(age_phrase(Some(3600)), "1h ago");
        assert_eq!(age_phrase(Some(86400)), "24h ago");
    }

    #[test]
    fn age_phrase_handles_never() {
        assert_eq!(age_phrase(None), "never");
    }

    #[test]
    fn zero_to_none_converts_only_zero() {
        assert_eq!(zero_to_none(0), None);
        assert_eq!(zero_to_none(1), Some(1));
        assert_eq!(zero_to_none(-1), Some(-1));
    }

    #[test]
    fn expected_platforms_derives_from_minimal_toml() {
        let cfg: crate::config::StonkwatchSocialConfig = toml::from_str(
            r#"
turso_url = "test"
turso_auth_token = ""
auto_post = true

[x]
client_id = ""
client_secret = ""
access_token = ""
refresh_token = ""

[bluesky]
identifier = ""
password = ""

[meta]
page_access_token = ""
page_id = ""
ig_user_id = "ig"
threads_user_id = "th"
threads_access_token = "tok"
"#,
        )
        .expect("test toml should parse");
        let mut platforms = expected_platforms_from_config(&cfg);
        platforms.sort();
        assert_eq!(
            platforms,
            vec!["bluesky", "facebook", "instagram", "instagram_story", "threads", "x"]
        );
    }

    #[test]
    fn expected_platforms_skips_meta_when_disabled() {
        let cfg: crate::config::StonkwatchSocialConfig = toml::from_str(
            r#"
turso_url = "test"
turso_auth_token = ""
auto_post = true

[meta]
page_access_token = ""
page_id = ""
facebook_enabled = false
instagram_enabled = false
threads_enabled = false
"#,
        )
        .expect("test toml should parse");
        assert!(expected_platforms_from_config(&cfg).is_empty());
    }

    #[test]
    fn expected_platforms_empty_when_no_socials_configured() {
        let cfg: crate::config::StonkwatchSocialConfig = toml::from_str(
            r#"
turso_url = "test"
turso_auth_token = ""
auto_post = true
"#,
        )
        .expect("test toml should parse");
        assert!(expected_platforms_from_config(&cfg).is_empty());
    }

    #[test]
    fn expected_platforms_empty_when_auto_post_disabled() {
        let cfg: crate::config::StonkwatchSocialConfig = toml::from_str(
            r#"
turso_url = "test"
turso_auth_token = ""
auto_post = false
review_room_id = "!review:matrix.org"

[x]
client_id = ""
client_secret = ""
access_token = ""
refresh_token = ""

[bluesky]
identifier = ""
password = ""
"#,
        )
        .expect("test toml should parse");
        assert!(
            expected_platforms_from_config(&cfg).is_empty(),
            "review-only deployments must not raise silence alerts"
        );
    }

    #[test]
    fn expected_platforms_includes_threads_without_access_token() {
        // build_platforms instantiates ThreadsPlatform whenever threads_enabled +
        // threads_user_id are present; the watchdog must surface the silence in
        // that misconfigured state instead of hiding it.
        let cfg: crate::config::StonkwatchSocialConfig = toml::from_str(
            r#"
turso_url = "test"
turso_auth_token = ""
auto_post = true

[meta]
page_access_token = ""
page_id = ""
facebook_enabled = false
instagram_enabled = false
threads_user_id = "th"
"#,
        )
        .expect("test toml should parse");
        assert_eq!(expected_platforms_from_config(&cfg), vec!["threads"]);
    }

    #[test]
    fn expected_platforms_excludes_tiktok_without_chart_video_api() {
        // TikTokPlatform.accepts() returns false when video_mp4_path is None,
        // and chart_video_api_base is what populates that path. Without it,
        // every legacy-timer post skips TikTok by design — no silence alert.
        let cfg: crate::config::StonkwatchSocialConfig = toml::from_str(
            r#"
turso_url = "test"
turso_auth_token = ""
auto_post = true

[tiktok]
client_key = ""
client_secret = ""
access_token = ""
refresh_token = ""
open_id = ""
"#,
        )
        .expect("test toml should parse");
        assert!(expected_platforms_from_config(&cfg).is_empty());
    }

    #[test]
    fn expected_platforms_includes_tiktok_when_chart_video_api_set() {
        let cfg: crate::config::StonkwatchSocialConfig = toml::from_str(
            r#"
turso_url = "test"
turso_auth_token = ""
auto_post = true
chart_video_api_base = "https://api.stonkwatch.app"

[tiktok]
client_key = ""
client_secret = ""
access_token = ""
refresh_token = ""
open_id = ""
"#,
        )
        .expect("test toml should parse");
        assert_eq!(expected_platforms_from_config(&cfg), vec!["tiktok"]);
    }
}
