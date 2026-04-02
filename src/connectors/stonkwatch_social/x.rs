use anyhow::{Context, Result};
use std::future::Future;
use std::pin::Pin;

use crate::config::XConfig;
use super::platform::{SocialPlatform, SocialPost, PostResult, check_response};

pub struct XPlatform {
    config: XConfig,
}

impl XPlatform {
    pub fn new(config: XConfig) -> Self {
        Self { config }
    }
}

impl SocialPlatform for XPlatform {
    fn name(&self) -> &'static str {
        "x"
    }

    fn publish<'a>(&'a self, post: &'a SocialPost) -> Pin<Box<dyn Future<Output = Result<PostResult>> + Send + 'a>> {
        Box::pin(async move {
            let id = post_tweet(&self.config, &post.text).await?;
            Ok(PostResult { platform: "x", id })
        })
    }
}

pub async fn post_tweet(config: &XConfig, text: &str) -> Result<String> {
    let client = reqwest::Client::new();

    let response = post_with_token(&client, text, &config.access_token).await?;

    if response.status().as_u16() == 401 {
        if let Some(ref refresh) = config.refresh_token {
            tracing::warn!(platform = "x", "Access token expired, attempting refresh");
            let new_token = refresh_access_token(&client, config, refresh).await?;
            let retry = post_with_token(&client, text, &new_token).await?;
            return parse_tweet_id(retry).await;
        }
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("X API auth error 401: {} — token may need re-authorization", body);
    }

    parse_tweet_id(response).await
}

async fn post_with_token(client: &reqwest::Client, text: &str, token: &str) -> Result<reqwest::Response> {
    let body = serde_json::json!({ "text": text });

    client
        .post("https://api.twitter.com/2/tweets")
        .header("Authorization", format!("Bearer {}", token))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .context("Failed to send tweet")
}

async fn parse_tweet_id(response: reqwest::Response) -> Result<String> {
    let response = check_response(response, "X").await?;
    let result: serde_json::Value = response.json().await?;
    result["data"]["id"]
        .as_str()
        .map(|s| s.to_string())
        .context("Missing 'data.id' in X API response")
}

async fn refresh_access_token(client: &reqwest::Client, config: &XConfig, refresh_token: &str) -> Result<String> {
    let response = client
        .post("https://api.twitter.com/2/oauth2/token")
        .basic_auth(&config.client_id, Some(&config.client_secret))
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
        ])
        .send()
        .await
        .context("Failed to refresh X access token")?;

    let response = check_response(response, "X token refresh").await?;
    let result: serde_json::Value = response.json().await?;
    result["access_token"]
        .as_str()
        .map(|s| s.to_string())
        .context("Missing 'access_token' in refresh response")
}
