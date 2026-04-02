use anyhow::{Context, Result};

use crate::config::XConfig;
use super::content::PostCandidate;

pub async fn post_tweet(config: &XConfig, post: &PostCandidate) -> Result<String> {
    let client = reqwest::Client::new();

    let response = post_with_token(&client, &post.text, &config.access_token).await?;

    if response.status().as_u16() == 401 {
        if let Some(ref refresh) = config.refresh_token {
            tracing::warn!(platform = "x", "Access token expired, attempting refresh");
            let new_token = refresh_access_token(&client, config, refresh).await?;
            let retry = post_with_token(&client, &post.text, &new_token).await?;
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
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("X API error {}: {}", status, body);
    }

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

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("X token refresh error {}: {}", status, body);
    }

    let result: serde_json::Value = response.json().await?;
    result["access_token"]
        .as_str()
        .map(|s| s.to_string())
        .context("Missing 'access_token' in refresh response")
}
