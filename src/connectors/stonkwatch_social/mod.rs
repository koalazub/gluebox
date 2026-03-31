pub mod content;
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

        loop {
            info!("Stonkwatch social: checking for content to post");

            match content::fetch_post_candidates(&config).await {
                Ok(candidates) => {
                    let to_post: Vec<_> = candidates.into_iter().take(5).collect();
                    if to_post.is_empty() {
                        info!("No notable events to post about");
                    } else {
                        info!("Posting {} updates across platforms", to_post.len());
                        for post in &to_post {
                            Self::post_to_all_platforms(&config, post).await;
                            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                        }
                    }
                }
                Err(e) => {
                    error!(error = %e, "Failed to fetch post candidates");
                }
            }

            tokio::time::sleep(std::time::Duration::from_secs(interval_secs)).await;
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
