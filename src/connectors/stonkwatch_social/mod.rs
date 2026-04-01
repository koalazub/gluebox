pub mod content;
pub mod og_image;
pub mod x;
pub mod bluesky;
pub mod instagram;
pub mod facebook;

use std::any::Any;
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
        let (approve_tx, mut approve_rx) = tokio::sync::mpsc::channel::<String>(32);

        if let Some(ref room_id) = config.review_room_id {
            info!(room_id, "Review mode enabled — posts will be sent to Matrix for approval");
        }

        loop {
            info!("Stonkwatch social: checking for content to post");

            match content::fetch_post_candidates(&config).await {
                Ok(candidates) => {
                    let to_post: Vec<_> = candidates.into_iter().take(5).collect();
                    if to_post.is_empty() {
                        info!("No notable events to post about");
                    } else if config.auto_post {
                        info!("Auto-posting {} updates across platforms", to_post.len());
                        for post in &to_post {
                            Self::post_to_all_platforms(&config, post).await;
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
                        }
                    } else {
                        info!("auto_post=false and no review_room_id — {} posts generated but not sent", to_post.len());
                    }
                }
                Err(e) => {
                    error!(error = %e, "Failed to fetch post candidates");
                }
            }

            tokio::time::sleep(std::time::Duration::from_secs(interval_secs)).await;
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

    async fn post_to_all_platforms(config: &StonkwatchSocialConfig, post: &content::PostCandidate) {
        if let Some(ref x_cfg) = config.x {
            match x::post_tweet(x_cfg, &post.text).await {
                Ok(id) => info!(platform = "x", tweet_id = %id, "Posted"),
                Err(e) => warn!(platform = "x", error = %e, "Failed to post"),
            }
        }

        if let Some(ref bsky_cfg) = config.bluesky {
            match bluesky::post(bsky_cfg, &post.text).await {
                Ok(uri) => info!(platform = "bluesky", uri = %uri, "Posted"),
                Err(e) => warn!(platform = "bluesky", error = %e, "Failed to post"),
            }
        }

        if let Some(ref ig_cfg) = config.instagram {
            match instagram::post(ig_cfg, &post.text, post.image_url.as_deref()).await {
                Ok(id) => info!(platform = "instagram", post_id = %id, "Posted"),
                Err(e) => warn!(platform = "instagram", error = %e, "Failed to post"),
            }
        }

        if let Some(ref fb_cfg) = config.facebook {
            match facebook::post(fb_cfg, &post.text).await {
                Ok(id) => info!(platform = "facebook", post_id = %id, "Posted"),
                Err(e) => warn!(platform = "facebook", error = %e, "Failed to post"),
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
