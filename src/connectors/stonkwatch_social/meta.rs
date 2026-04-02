use anyhow::{Context, Result};
use std::future::Future;
use std::pin::Pin;

use crate::config::MetaConfig;
use super::platform::{SocialPlatform, SocialPost, PostResult, check_response};

fn strip_url(text: &str) -> String {
    text.split_whitespace()
        .filter(|w| !w.starts_with("https://") && !w.starts_with("http://"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn instagram_caption(post: &SocialPost) -> String {
    let clean = strip_url(&post.text);
    format!("{}\n\nFull analysis on stonkwatch.app", clean.trim())
}

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
            let client = reqwest::Client::new();

            let mut params = vec![
                ("message", post.text.as_str()),
                ("access_token", self.config.page_access_token.as_str()),
            ];

            if !post.link.is_empty() {
                params.push(("link", post.link.as_str()));
            }

            let response = client
                .post(format!("https://graph.facebook.com/v25.0/{}/feed", self.config.page_id))
                .form(&params)
                .send()
                .await
                .context("Failed to post to Facebook")?;

            let response = check_response(response, "Facebook").await?;
            let result: serde_json::Value = response.json().await?;
            let id = result["id"]
                .as_str()
                .map(|s| s.to_string())
                .context("Missing 'id' in Facebook response")?;

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
            let client = reqwest::Client::new();

            let ig_user_id = self.config.ig_user_id.as_deref()
                .context("ig_user_id required for Instagram posting")?;

            let image_url = post.image_url.as_deref()
                .context("Instagram requires a public image URL")?;

            let caption = instagram_caption(post);

            let container_response = client
                .post(format!("https://graph.facebook.com/v25.0/{}/media", ig_user_id))
                .form(&[
                    ("image_url", image_url),
                    ("caption", caption.as_str()),
                    ("access_token", self.config.page_access_token.as_str()),
                ])
                .send()
                .await
                .context("Failed to create Instagram media container")?;

            let container_response = check_response(container_response, "Instagram container").await?;
            let container: serde_json::Value = container_response.json().await?;
            let container_id = container["id"]
                .as_str()
                .context("Missing container ID in Instagram response")?
                .to_string();

            tokio::time::sleep(std::time::Duration::from_secs(10)).await;

            let publish_response = client
                .post(format!("https://graph.facebook.com/v25.0/{}/media_publish", ig_user_id))
                .form(&[
                    ("creation_id", container_id.as_str()),
                    ("access_token", self.config.page_access_token.as_str()),
                ])
                .send()
                .await
                .context("Failed to publish Instagram post")?;

            let publish_response = check_response(publish_response, "Instagram publish").await?;
            let result: serde_json::Value = publish_response.json().await?;
            let id = result["id"]
                .as_str()
                .map(|s| s.to_string())
                .context("Missing 'id' in Instagram response")?;

            Ok(PostResult { platform: "instagram", id })
        })
    }
}

pub struct InstagramStoryPlatform {
    config: MetaConfig,
}

impl InstagramStoryPlatform {
    pub fn new(config: MetaConfig) -> Self {
        Self { config }
    }
}

impl SocialPlatform for InstagramStoryPlatform {
    fn name(&self) -> &'static str {
        "instagram_story"
    }

    fn publish<'a>(&'a self, post: &'a SocialPost) -> Pin<Box<dyn Future<Output = Result<PostResult>> + Send + 'a>> {
        Box::pin(async move {
            let client = reqwest::Client::new();

            let ig_user_id = self.config.ig_user_id.as_deref()
                .context("ig_user_id required for Instagram Stories")?;

            let image_url = post.story_image_url.as_deref()
                .or(post.image_url.as_deref())
                .context("Instagram Stories requires a public image URL")?;

            let container_response = client
                .post(format!("https://graph.facebook.com/v25.0/{}/media", ig_user_id))
                .form(&[
                    ("image_url", image_url),
                    ("media_type", "STORIES"),
                    ("access_token", self.config.page_access_token.as_str()),
                ])
                .send()
                .await
                .context("Failed to create Instagram Story container")?;

            let container_response = check_response(container_response, "Instagram Story container").await?;
            let container: serde_json::Value = container_response.json().await?;
            let container_id = container["id"]
                .as_str()
                .context("Missing container ID in Instagram Story response")?
                .to_string();

            tokio::time::sleep(std::time::Duration::from_secs(10)).await;

            let publish_response = client
                .post(format!("https://graph.facebook.com/v25.0/{}/media_publish", ig_user_id))
                .form(&[
                    ("creation_id", container_id.as_str()),
                    ("access_token", self.config.page_access_token.as_str()),
                ])
                .send()
                .await
                .context("Failed to publish Instagram Story")?;

            let publish_response = check_response(publish_response, "Instagram Story publish").await?;
            let result: serde_json::Value = publish_response.json().await?;
            let id = result["id"]
                .as_str()
                .map(|s| s.to_string())
                .context("Missing 'id' in Instagram Story response")?;

            Ok(PostResult { platform: "instagram_story", id })
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
            let client = reqwest::Client::new();

            let threads_user_id = self.config.threads_user_id.as_deref()
                .context("threads_user_id required for Threads posting")?;

            let threads_token = self.config.threads_access_token.as_deref()
                .unwrap_or(&self.config.page_access_token);

            let container_response = client
                .post(format!("https://graph.threads.net/v1.0/{}/threads", threads_user_id))
                .form(&[
                    ("media_type", "TEXT"),
                    ("text", post.text.as_str()),
                    ("access_token", threads_token),
                ])
                .send()
                .await
                .context("Failed to create Threads container")?;

            let container_response = check_response(container_response, "Threads container").await?;
            let container: serde_json::Value = container_response.json().await?;
            let container_id = container["id"]
                .as_str()
                .context("Missing container ID in Threads response")?
                .to_string();

            let publish_response = client
                .post(format!("https://graph.threads.net/v1.0/{}/threads_publish", threads_user_id))
                .form(&[
                    ("creation_id", container_id.as_str()),
                    ("access_token", threads_token),
                ])
                .send()
                .await
                .context("Failed to publish Threads post")?;

            let publish_response = check_response(publish_response, "Threads publish").await?;
            let result: serde_json::Value = publish_response.json().await?;
            let id = result["id"]
                .as_str()
                .map(|s| s.to_string())
                .context("Missing 'id' in Threads response")?;

            Ok(PostResult { platform: "threads", id })
        })
    }
}
