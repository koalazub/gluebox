use anyhow::{Context, Result};

use crate::config::InstagramConfig;

pub async fn post(config: &InstagramConfig, caption: &str, image_url: Option<&str>) -> Result<String> {
    let client = reqwest::Client::new();

    let image = image_url.unwrap_or(&config.default_image_url);

    let container_response = client
        .post(format!(
            "https://graph.facebook.com/v21.0/{}/media",
            config.ig_user_id
        ))
        .form(&[
            ("image_url", image),
            ("caption", caption),
            ("access_token", &config.access_token),
        ])
        .send()
        .await
        .context("Failed to create Instagram media container")?;

    if !container_response.status().is_success() {
        let status = container_response.status();
        let body = container_response.text().await.unwrap_or_default();
        anyhow::bail!("Instagram container creation error {}: {}", status, body);
    }

    let container: serde_json::Value = container_response.json().await?;
    let container_id = container["id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("No container ID in Instagram response"))?;

    tokio::time::sleep(std::time::Duration::from_secs(10)).await;

    let publish_response = client
        .post(format!(
            "https://graph.facebook.com/v21.0/{}/media_publish",
            config.ig_user_id
        ))
        .form(&[
            ("creation_id", container_id),
            ("access_token", &config.access_token),
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
    Ok(result["id"].as_str().unwrap_or("unknown").to_string())
}
