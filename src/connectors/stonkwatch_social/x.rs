use anyhow::{Context, Result};

use crate::config::XConfig;

pub async fn post_tweet(config: &XConfig, text: &str) -> Result<String> {
    let client = reqwest::Client::new();
    let body = serde_json::json!({ "text": text });

    let response = client
        .post("https://api.twitter.com/2/tweets")
        .header("Authorization", format!("Bearer {}", config.access_token))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .context("Failed to send tweet")?;

    if response.status().as_u16() == 401 {
        if let Some(ref refresh) = config.refresh_token {
            tracing::warn!(platform = "x", "Access token expired, attempting refresh");
            let new_token = refresh_access_token(config, refresh).await?;
            return post_with_token(&client, text, &new_token).await;
        }
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("X API auth error 401: {} — token may need re-authorization", body);
    }

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("X API error {}: {}", status, body);
    }

    let result: serde_json::Value = response.json().await?;
    Ok(result["data"]["id"].as_str().unwrap_or("unknown").to_string())
}

async fn post_with_token(client: &reqwest::Client, text: &str, token: &str) -> Result<String> {
    let body = serde_json::json!({ "text": text });

    let response = client
        .post("https://api.twitter.com/2/tweets")
        .header("Authorization", format!("Bearer {}", token))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .context("Failed to send tweet with refreshed token")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("X API error {}: {}", status, body);
    }

    let result: serde_json::Value = response.json().await?;
    Ok(result["data"]["id"].as_str().unwrap_or("unknown").to_string())
}

async fn refresh_access_token(config: &XConfig, refresh_token: &str) -> Result<String> {
    let client = reqwest::Client::new();

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
    let new_token = result["access_token"]
        .as_str()
        .context("Missing access_token in refresh response")?;

    tracing::info!(platform = "x", "Access token refreshed successfully");
    Ok(new_token.to_string())
}
