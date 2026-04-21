pub mod chart_video;
pub mod content;
pub mod og_image;
pub mod storj;
pub mod stitcher;
pub mod x;
pub mod bluesky;
pub mod meta;
pub mod tiktok;
pub mod platform;
pub mod pipeline;
pub mod replica;

use std::any::Any;
use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU8, Ordering};
use tokio::sync::Mutex;
use tracing::{error, info, warn};

use gluebox_core::{Connector, ConnectorStatus};
use crate::config::StonkwatchSocialConfig;
use platform::{SocialPlatform, SocialPost};

const PROMO_MESSAGES: &[(&str, &str)] = &[
    (
        "Track ASX announcements with AI-powered summaries, real-time alerts, and community discussion. Free to use.\n\nhttps://stonkwatch.app/feed",
        "https://stonkwatch.app/feed",
    ),
    (
        "Every ASX announcement, analysed by AI in seconds. No more reading 50-page PDFs.\n\nhttps://stonkwatch.app/register",
        "https://stonkwatch.app/register",
    ),
    (
        "New to Bluesky? Follow the ASX investing community — traders, analysts, and market watchers.\n\nhttps://bsky.app/starter-pack/stonkwatch.bsky.social",
        "https://go.bsky.app/34M6hf2",
    ),
];

fn is_asx_market_hours() -> bool {
    let now = chrono::Utc::now() + chrono::Duration::hours(10);
    let hour = now.hour();
    let weekday = now.weekday();
    matches!(weekday, chrono::Weekday::Mon | chrono::Weekday::Tue | chrono::Weekday::Wed | chrono::Weekday::Thu | chrono::Weekday::Fri)
        && (10..16).contains(&hour)
}

use chrono::Datelike;
use chrono::Timelike;

pub struct StonkwatchSocialConnector {
    config: Mutex<StonkwatchSocialConfig>,
    status: AtomicU8,
    error_msg: Mutex<Option<String>>,
    task_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl StonkwatchSocialConnector {
    pub fn new(config: StonkwatchSocialConfig) -> Self {
        Self {
            config: Mutex::new(config),
            status: AtomicU8::new(ConnectorStatus::Stopped.as_u8()),
            error_msg: Mutex::new(None),
            task_handle: Mutex::new(None),
        }
    }

    pub fn build_platforms(config: &StonkwatchSocialConfig) -> Vec<Box<dyn SocialPlatform>> {
        let mut platforms: Vec<Box<dyn SocialPlatform>> = Vec::new();

        if let Some(ref x_cfg) = config.x {
            platforms.push(Box::new(x::XPlatform::new(x_cfg.clone())));
        }

        if let Some(ref bsky_cfg) = config.bluesky {
            platforms.push(Box::new(bluesky::BlueskyPlatform::new(bsky_cfg.clone())));
        }

        if let Some(ref meta_cfg) = config.meta {
            if meta_cfg.facebook_enabled {
                platforms.push(Box::new(meta::FacebookPlatform::new(meta_cfg.clone())));
            }
            if meta_cfg.instagram_enabled && meta_cfg.ig_user_id.is_some() {
                platforms.push(Box::new(meta::InstagramPlatform::new(meta_cfg.clone())));
                platforms.push(Box::new(meta::InstagramStoryPlatform::new(meta_cfg.clone())));
            }
            if meta_cfg.threads_enabled && meta_cfg.threads_user_id.is_some() {
                if meta_cfg.threads_access_token.is_none() {
                    tracing::warn!("Threads enabled with threads_user_id but threads_access_token is missing — Threads posting will fail. Generate a Threads OAuth token at developers.facebook.com.");
                }
                platforms.push(Box::new(meta::ThreadsPlatform::new(meta_cfg.clone())));
            }
        }

        if let Some(ref tiktok_cfg) = config.tiktok {
            platforms.push(Box::new(tiktok::TikTokPlatform::new(tiktok_cfg.clone())));
        }

        platforms
    }

    async fn run_posting_loop(config: StonkwatchSocialConfig) {
        let interval_secs = config.post_interval_secs.unwrap_or(21600);
        let max_posts = config.max_posts_per_cycle.max(1) as usize;
        let platforms = Self::build_platforms(&config);

        Self::log_platform_status(&platforms, &config);

        info!(url = %config.turso_url, "Connecting to Stonkwatch Turso DB");

        let mut posted_ids: HashSet<String> = HashSet::new();
        let mut cycle_count: u64 = 0;
        let mut promo_index: usize = 0;

        loop {
            cycle_count += 1;
            info!("Stonkwatch social: checking for content to post");

            let db = match replica::open_synced_replica(&config).await {
                Ok(db) => db,
                Err(e) => {
                    error!(error = %e, "Failed to open synced Stonkwatch replica");
                    tokio::time::sleep(std::time::Duration::from_secs(interval_secs)).await;
                    continue;
                }
            };

            let conn = match db.connect().await {
                Ok(c) => c,
                Err(e) => {
                    error!(error = %e, "Failed to get Turso connection");
                    tokio::time::sleep(std::time::Duration::from_secs(interval_secs)).await;
                    continue;
                }
            };

            match content::fetch_post_candidates(&conn, &config, &posted_ids).await {
                Ok(candidates) => {
                    let filtered: Vec<_> = if config.trending_only {
                        candidates.into_iter().filter(|c| c.is_price_sensitive()).collect()
                    } else {
                        candidates
                    };
                    let to_post: Vec<_> = filtered.into_iter().take(max_posts).collect();
                    if to_post.is_empty() {
                        info!(trending_only = config.trending_only, "No new announcements to post about");
                    } else if config.auto_post {
                        info!("Auto-posting {} updates across platforms", to_post.len());
                        for post in &to_post {
                            let social_post = post.to_social_post();
                            Self::post_to_all_platforms(&platforms, &social_post).await;
                            posted_ids.insert(post.announcement_id.clone());
                            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                        }
                    } else if config.review_room_id.is_some() {
                        info!("Sending {} posts to Matrix for review", to_post.len());
                        for (i, post) in to_post.iter().enumerate() {
                            let preview = format!(
                                "📋 **Post {}/{}**\n\n{}\n\n_React ✅ to approve, ❌ to reject_",
                                i + 1,
                                to_post.len(),
                                post.text
                            );
                            Self::send_to_review(&config, &preview).await;
                            posted_ids.insert(post.announcement_id.clone());
                        }
                    } else {
                        info!("auto_post=false and no review_room_id — {} posts generated but not sent", to_post.len());
                    }
                }
                Err(e) => {
                    error!(error = %e, "Failed to fetch post candidates");
                }
            }

            if posted_ids.len() > 500 {
                posted_ids.clear();
            }

            let promo_interval = if is_asx_market_hours() { 6 } else { 12 };
            if cycle_count % promo_interval == 0 {
                let (text, link) = PROMO_MESSAGES[promo_index % PROMO_MESSAGES.len()];
                let promo = SocialPost {
                    text: text.to_string(),
                    link: link.to_string(),
                    image_url: None,
                    story_image_url: None,
                    og_title: "Stonkwatch — ASX Market Intelligence".to_string(),
                    og_description: "AI-powered ASX announcement summaries, sentiment tracking, and market analysis.".to_string(),
                    video_mp4_path: None,
                };
                info!("Posting promo message {}", promo_index + 1);
                Self::post_to_all_platforms(&platforms, &promo).await;
                promo_index += 1;
            }

            tokio::time::sleep(std::time::Duration::from_secs(interval_secs)).await;
        }
    }

    fn log_platform_status(platforms: &[Box<dyn SocialPlatform>], config: &StonkwatchSocialConfig) {
        for p in platforms {
            info!(platform = p.name(), "Configured");
        }

        if config.x.is_none() {
            warn!(platform = "x", "Not configured — posts will be skipped");
        }
        if config.bluesky.is_none() {
            warn!(platform = "bluesky", "Not configured — posts will be skipped");
        }
        if config.meta.is_none() {
            warn!(platform = "meta", "Not configured — Facebook/Instagram/Threads will be skipped");
        }

        if config.auto_post {
            info!("Auto-post enabled");
        } else if config.review_room_id.is_some() {
            info!(room_id = config.review_room_id.as_deref().unwrap_or(""), "Review mode enabled — posts will be sent to Matrix for approval");
        } else {
            warn!("auto_post=false and no review_room_id — posts will be generated but NOT sent anywhere");
        }
    }

    async fn send_to_review(config: &StonkwatchSocialConfig, message: &str) {
        let notify_url = std::env::var("GLUEBOX_URL")
            .or_else(|_| std::env::var("GLUEBOX_NOTIFY_URL"))
            .unwrap_or_else(|_| "http://127.0.0.1:8990".to_string());

        let notify_secret = std::env::var("GLUEBOX_NOTIFY_SECRET").unwrap_or_default();

        let room_id = match &config.review_room_id {
            Some(id) => id.clone(),
            None => return,
        };

        let client = reqwest::Client::new();
        let body = serde_json::json!({
            "room_id": room_id,
            "message": message,
        });

        match client
            .post(format!("{}/api/notify", notify_url))
            .header("Authorization", format!("Bearer {}", notify_secret))
            .json(&body)
            .send()
            .await
        {
            Ok(_) => info!("Sent post to Matrix for review"),
            Err(e) => warn!(error = %e, "Failed to send post to Matrix for review"),
        }
    }

    async fn post_to_all_platforms(
        platforms: &[Box<dyn SocialPlatform>],
        post: &SocialPost,
    ) {
        for platform in platforms {
            if !platform.accepts(post) {
                tracing::debug!(platform = platform.name(), "skipping platform: does not accept this post");
                continue;
            }
            match platform.publish(post).await {
                Ok(result) => info!(platform = result.platform, id = %result.id, "Posted"),
                Err(e) => error!(platform = platform.name(), error = %e, "Failed to post"),
            }
        }
    }
}

impl Connector for StonkwatchSocialConnector {
    fn name(&self) -> &'static str {
        "stonkwatch_social"
    }

    fn status(&self) -> ConnectorStatus {
        match self.status.load(Ordering::SeqCst) {
            0 => ConnectorStatus::Running,
            1 => ConnectorStatus::Stopped,
            2 => ConnectorStatus::Suspended,
            _ => {
                let msg = self.error_msg.blocking_lock().clone().unwrap_or_default();
                ConnectorStatus::Error(msg)
            }
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn start(&self) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>> {
        Box::pin(async move {
            let config = self.config.lock().await.clone();
            if config.trending_webhook_driven {
                info!("Stonkwatch social connector started in webhook-driven mode (no posting loop)");
            } else {
                let handle = tokio::spawn(Self::run_posting_loop(config));
                *self.task_handle.lock().await = Some(handle);
                info!("Stonkwatch social connector started (legacy timer loop)");
            }
            self.status.store(ConnectorStatus::Running.as_u8(), Ordering::SeqCst);
            Ok(())
        })
    }

    fn stop(&self) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>> {
        Box::pin(async move {
            if let Some(handle) = self.task_handle.lock().await.take() {
                handle.abort();
            }
            self.status.store(ConnectorStatus::Stopped.as_u8(), Ordering::SeqCst);
            info!("Stonkwatch social connector stopped");
            Ok(())
        })
    }

    fn suspend(&self) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>> {
        Box::pin(async { Ok(()) })
    }

    fn resume(&self) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>> {
        Box::pin(async { Ok(()) })
    }

    fn health_check(&self) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>> {
        Box::pin(async move {
            if self.task_handle.lock().await.is_some() {
                Ok(())
            } else {
                anyhow::bail!("stonkwatch social connector not running")
            }
        })
    }

    fn reconfigure(
        &self,
        raw_toml: &toml::Value,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<bool>> + Send + '_>> {
        let raw = raw_toml.clone();
        Box::pin(async move {
            let new_config: StonkwatchSocialConfig = raw.try_into()?;
            *self.config.lock().await = new_config;
            Ok(false)
        })
    }
}
