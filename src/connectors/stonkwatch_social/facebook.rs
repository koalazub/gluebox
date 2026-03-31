use anyhow::{Context, Result};

use crate::config::FacebookConfig;

pub async fn post(config: &FacebookConfig, message: &str) -> Result<String> {
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
    Ok(result["id"].as_str().unwrap_or("unknown").to_string())
}
