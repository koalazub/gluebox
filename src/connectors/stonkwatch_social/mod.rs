pub mod content;
pub mod og_image;
pub mod x;
pub mod bluesky;
pub mod meta;

use std::any::Any;
use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU8, Ordering};
use tokio::sync::Mutex;
use tracing::{error, info, warn};

use crate::connector::{Connector, ConnectorStatus};
use crate::config::StonkwatchSocialConfig;

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

    async fn run_posting_loop(config: StonkwatchSocialConfig) {
        let interval_secs = config.post_interval_secs.unwrap_or(14400);

        Self::log_platform_status(&config);

        let db = match turso::sync::Builder::new_remote("/var/lib/gluebox/stonkwatch-replica")
            .with_remote_url(&config.turso_url)
            .with_auth_token(&config.turso_auth_token)
            .build()
            .await
        {
            Ok(db) => db,
            Err(e) => {
                error!(error = %e, "Failed to connect to Stonkwatch Turso DB — social connector stopping");
                return;
            }
        };

        info!(url = %config.turso_url, "Connected to Stonkwatch Turso DB");

        let mut posted_ids: HashSet<String> = HashSet::new();
        let mut bsky_session: Option<bluesky::CachedSession> = None;

        loop {
            info!("Stonkwatch social: checking for content to post");

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
                    let to_post: Vec<_> = candidates.into_iter().take(5).collect();
                    if to_post.is_empty() {
                        info!("No new announcements to post about");
                    } else if config.auto_post {
                        info!("Auto-posting {} updates across platforms", to_post.len());
                        for post in &to_post {
                            Self::post_to_all_platforms(&config, post, &mut bsky_session).await;
                            posted_ids.insert(post.id.clone());
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
                            posted_ids.insert(post.id.clone());
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

            tokio::time::sleep(std::time::Duration::from_secs(interval_secs)).await;
        }
    }

    fn log_platform_status(config: &StonkwatchSocialConfig) {
        if config.x.is_some() {
            info!(platform = "x", "Configured");
        } else {
            warn!(platform = "x", "Not configured — posts will be skipped");
        }
        if config.bluesky.is_some() {
            info!(platform = "bluesky", "Configured");
        } else {
            warn!(platform = "bluesky", "Not configured — posts will be skipped");
        }
        if let Some(ref meta) = config.meta {
            if meta.facebook_enabled { info!(platform = "facebook", "Configured via Meta"); }
            if meta.instagram_enabled && meta.ig_user_id.is_some() { info!(platform = "instagram", "Configured via Meta"); }
            if meta.threads_enabled && meta.threads_user_id.is_some() { info!(platform = "threads", "Configured via Meta"); }
        } else {
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
        config: &StonkwatchSocialConfig,
        post: &content::PostCandidate,
        bsky_session: &mut Option<bluesky::CachedSession>,
    ) {
        if let Some(ref x_cfg) = config.x {
            match x::post_tweet(x_cfg, &post.text).await {
                Ok(id) => info!(platform = "x", tweet_id = %id, "Posted"),
                Err(e) => error!(platform = "x", error = %e, "Failed to post"),
            }
        }

        if let Some(ref bsky_cfg) = config.bluesky {
            match bluesky::post_with_session(bsky_cfg, &post.text, bsky_session).await {
                Ok(uri) => info!(platform = "bluesky", uri = %uri, "Posted"),
                Err(e) => error!(platform = "bluesky", error = %e, "Failed to post"),
            }
        }

        if let Some(ref meta_cfg) = config.meta {
            for (platform, result) in meta::post_all(meta_cfg, &post.text, post.image_url.as_deref()).await {
                match result {
                    Ok(id) => info!(%platform, post_id = %id, "Posted"),
                    Err(e) => error!(%platform, error = %e, "Failed to post"),
                }
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
            let handle = tokio::spawn(Self::run_posting_loop(config));
            *self.task_handle.lock().await = Some(handle);
            self.status.store(ConnectorStatus::Running.as_u8(), Ordering::SeqCst);
            info!("Stonkwatch social connector started");
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
