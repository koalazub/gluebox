use anyhow::{Context, Result};
use tracing::info;

use crate::config::MetaConfig;

pub async fn post_to_facebook(config: &MetaConfig, message: &str) -> Result<String> {
    let client = reqwest::Client::new();

    let response = client
        .post(format!(
            "https://graph.facebook.com/v21.0/{}/feed",
            config.page_id
        ))
        .form(&[
            ("message", message),
            ("access_token", &config.page_access_token),
        ])
        .send()
        .await
        .context("Failed to post to Facebook")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("Facebook API error {}: {}", status, body);
    }

    let result: serde_json::Value = response.json().await?;
    let id = result["id"].as_str().context("Missing 'id' in Facebook response")?;
    Ok(id.to_string())
}

pub async fn post_to_instagram(config: &MetaConfig, caption: &str, image_url: &str) -> Result<String> {
    let client = reqwest::Client::new();

    let ig_user_id = config.ig_user_id.as_deref()
        .context("ig_user_id required for Instagram posting")?;

    let container_response = client
        .post(format!(
            "https://graph.facebook.com/v21.0/{}/media",
            ig_user_id
        ))
        .form(&[
            ("image_url", image_url),
            ("caption", caption),
            ("access_token", &config.page_access_token),
        ])
        .send()
        .await
        .context("Failed to create Instagram media container")?;

    if !container_response.status().is_success() {
        let status = container_response.status();
        let body = container_response.text().await.unwrap_or_default();
        anyhow::bail!("Instagram container error {}: {}", status, body);
    }

    let container: serde_json::Value = container_response.json().await?;
    let container_id = container["id"]
        .as_str()
        .context("Missing container ID in Instagram response")?;

    tokio::time::sleep(std::time::Duration::from_secs(10)).await;

    let publish_response = client
        .post(format!(
            "https://graph.facebook.com/v21.0/{}/media_publish",
            ig_user_id
        ))
        .form(&[
            ("creation_id", container_id),
            ("access_token", &config.page_access_token),
        ])
        .send()
        .await
        .context("Failed to publish Instagram post")?;

    if !publish_response.status().is_success() {
        let status = publish_response.status();
        let body = publish_response.text().await.unwrap_or_default();
        anyhow::bail!("Instagram publish error {}: {}", status, body);
    }

    let result: serde_json::Value = publish_response.json().await?;
    let id = result["id"].as_str().context("Missing 'id' in Instagram response")?;
    Ok(id.to_string())
}

pub async fn post_to_threads(config: &MetaConfig, text: &str) -> Result<String> {
    let client = reqwest::Client::new();

    let threads_user_id = config.threads_user_id.as_deref()
        .context("threads_user_id required for Threads posting")?;

    let container_response = client
        .post(format!(
            "https://graph.threads.net/v1.0/{}/threads",
            threads_user_id
        ))
        .form(&[
            ("media_type", "TEXT"),
            ("text", text),
            ("access_token", &config.page_access_token),
        ])
        .send()
        .await
        .context("Failed to create Threads container")?;

    if !container_response.status().is_success() {
        let status = container_response.status();
        let body = container_response.text().await.unwrap_or_default();
        anyhow::bail!("Threads container error {}: {}", status, body);
    }

    let container: serde_json::Value = container_response.json().await?;
    let container_id = container["id"]
        .as_str()
        .context("Missing container ID in Threads response")?;

    let publish_response = client
        .post(format!(
            "https://graph.threads.net/v1.0/{}/threads_publish",
            threads_user_id
        ))
        .form(&[
            ("creation_id", container_id),
            ("access_token", &config.page_access_token),
        ])
        .send()
        .await
        .context("Failed to publish Threads post")?;

    if !publish_response.status().is_success() {
        let status = publish_response.status();
        let body = publish_response.text().await.unwrap_or_default();
        anyhow::bail!("Threads publish error {}: {}", status, body);
    }

    let result: serde_json::Value = publish_response.json().await?;
    let id = result["id"].as_str().context("Missing 'id' in Threads response")?;
    Ok(id.to_string())
}

pub async fn post_all(config: &MetaConfig, text: &str, image_url: Option<&str>) -> Vec<(&'static str, Result<String>)> {
    let mut results = Vec::new();

    if config.facebook_enabled {
        results.push(("facebook", post_to_facebook(config, text).await));
    }

    if config.instagram_enabled && config.ig_user_id.is_some() {
        if let Some(img) = image_url {
            results.push(("instagram", post_to_instagram(config, text, img).await));
        } else {
            info!(platform = "instagram", "Skipped — no image URL available");
        }
    }

    if config.threads_enabled && config.threads_user_id.is_some() {
        results.push(("threads", post_to_threads(config, text).await));
    }

    results
}
