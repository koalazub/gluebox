use anyhow::{Context, Result};
use std::future::Future;
use std::pin::Pin;

use crate::config::MetaConfig;
use super::platform::{SocialPlatform, SocialPost, PostResult, check_response};

pub struct FacebookPlatform {
    config: MetaConfig,
}

impl FacebookPlatform {
    pub fn new(config: MetaConfig) -> Self {
        Self { config }
    }
}

impl SocialPlatform for FacebookPlatform {
    fn name(&self) -> &'static str {
        "facebook"
    }

    fn publish<'a>(&'a self, post: &'a SocialPost) -> Pin<Box<dyn Future<Output = Result<PostResult>> + Send + 'a>> {
        Box::pin(async move {
            let id = post_to_facebook(&self.config, &post.text).await?;
            Ok(PostResult { platform: "facebook", id })
        })
    }
}

pub struct InstagramPlatform {
    config: MetaConfig,
}

impl InstagramPlatform {
    pub fn new(config: MetaConfig) -> Self {
        Self { config }
    }
}

impl SocialPlatform for InstagramPlatform {
    fn name(&self) -> &'static str {
        "instagram"
    }

    fn publish<'a>(&'a self, post: &'a SocialPost) -> Pin<Box<dyn Future<Output = Result<PostResult>> + Send + 'a>> {
        Box::pin(async move {
            let id = post_to_instagram(&self.config, &post.text, post.image_url.as_deref()).await?;
            Ok(PostResult { platform: "instagram", id })
        })
    }
}

pub struct ThreadsPlatform {
    config: MetaConfig,
}

impl ThreadsPlatform {
    pub fn new(config: MetaConfig) -> Self {
        Self { config }
    }
}

impl SocialPlatform for ThreadsPlatform {
    fn name(&self) -> &'static str {
        "threads"
    }

    fn publish<'a>(&'a self, post: &'a SocialPost) -> Pin<Box<dyn Future<Output = Result<PostResult>> + Send + 'a>> {
        Box::pin(async move {
            let id = post_to_threads(&self.config, &post.text).await?;
            Ok(PostResult { platform: "threads", id })
        })
    }
}

async fn post_to_facebook(config: &MetaConfig, text: &str) -> Result<String> {
    let client = reqwest::Client::new();

    let response = client
        .post(format!("https://graph.facebook.com/v21.0/{}/feed", config.page_id))
        .form(&[
            ("message", text),
            ("access_token", config.page_access_token.as_str()),
        ])
        .send()
        .await
        .context("Failed to post to Facebook")?;

    let response = check_response(response, "Facebook").await?;
    let result: serde_json::Value = response.json().await?;
    result["id"]
        .as_str()
        .map(|s| s.to_string())
        .context("Missing 'id' in Facebook response")
}

async fn post_to_instagram(config: &MetaConfig, text: &str, image_url: Option<&str>) -> Result<String> {
    let client = reqwest::Client::new();

    let ig_user_id = config.ig_user_id.as_deref()
        .context("ig_user_id required for Instagram posting")?;

    let image_url = image_url
        .context("Instagram requires an image URL")?;

    let container_response = client
        .post(format!("https://graph.facebook.com/v21.0/{}/media", ig_user_id))
        .form(&[
            ("image_url", image_url),
            ("caption", text),
            ("access_token", config.page_access_token.as_str()),
        ])
        .send()
        .await
        .context("Failed to create Instagram media container")?;

    let container_response = check_response(container_response, "Instagram container").await?;
    let container: serde_json::Value = container_response.json().await?;
    let container_id = container["id"]
        .as_str()
        .context("Missing container ID in Instagram response")?;

    tokio::time::sleep(std::time::Duration::from_secs(10)).await;

    let publish_response = client
        .post(format!("https://graph.facebook.com/v21.0/{}/media_publish", ig_user_id))
        .form(&[
            ("creation_id", container_id),
            ("access_token", config.page_access_token.as_str()),
        ])
        .send()
        .await
        .context("Failed to publish Instagram post")?;

    let publish_response = check_response(publish_response, "Instagram publish").await?;
    let result: serde_json::Value = publish_response.json().await?;
    result["id"]
        .as_str()
        .map(|s| s.to_string())
        .context("Missing 'id' in Instagram response")
}

async fn post_to_threads(config: &MetaConfig, text: &str) -> Result<String> {
    let client = reqwest::Client::new();

    let threads_user_id = config.threads_user_id.as_deref()
        .context("threads_user_id required for Threads posting")?;

    let container_response = client
        .post(format!("https://graph.threads.net/v1.0/{}/threads", threads_user_id))
        .form(&[
            ("media_type", "TEXT"),
            ("text", text),
            ("access_token", config.page_access_token.as_str()),
        ])
        .send()
        .await
        .context("Failed to create Threads container")?;

    let container_response = check_response(container_response, "Threads container").await?;
    let container: serde_json::Value = container_response.json().await?;
    let container_id = container["id"]
        .as_str()
        .context("Missing container ID in Threads response")?;

    let publish_response = client
        .post(format!("https://graph.threads.net/v1.0/{}/threads_publish", threads_user_id))
        .form(&[
            ("creation_id", container_id),
            ("access_token", config.page_access_token.as_str()),
        ])
        .send()
        .await
        .context("Failed to publish Threads post")?;

    let publish_response = check_response(publish_response, "Threads publish").await?;
    let result: serde_json::Value = publish_response.json().await?;
    result["id"]
        .as_str()
        .map(|s| s.to_string())
        .context("Missing 'id' in Threads response")
}
