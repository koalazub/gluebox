use anyhow::{Context, Result};
use chrono::Utc;

use crate::config::BlueskyConfig;

pub async fn post(config: &BlueskyConfig, text: &str) -> Result<String> {
    let client = reqwest::Client::new();

    let session = create_session(&client, &config.identifier, &config.password).await?;

    let record = serde_json::json!({
        "repo": session.did,
        "collection": "app.bsky.feed.post",
        "record": {
            "$type": "app.bsky.feed.post",
            "text": text,
            "createdAt": Utc::now().to_rfc3339(),
            "langs": ["en"],
        }
    });

    let response = client
        .post(format!("{}/xrpc/com.atproto.repo.createRecord", config.service_url))
        .header("Authorization", format!("Bearer {}", session.access_jwt))
        .json(&record)
        .send()
        .await
        .context("Failed to create Bluesky post")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("Bluesky API error {}: {}", status, body);
    }

    let result: serde_json::Value = response.json().await?;
    Ok(result["uri"].as_str().unwrap_or("unknown").to_string())
}

#[derive(serde::Deserialize)]
struct Session {
    did: String,
    #[serde(rename = "accessJwt")]
    access_jwt: String,
}

async fn create_session(client: &reqwest::Client, identifier: &str, password: &str) -> Result<Session> {
    let body = serde_json::json!({
        "identifier": identifier,
        "password": password,
    });

    let response = client
        .post("https://bsky.social/xrpc/com.atproto.server.createSession")
        .json(&body)
        .send()
        .await
        .context("Failed to create Bluesky session")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("Bluesky auth error {}: {}", status, body);
    }

    response.json().await.context("Failed to parse Bluesky session")
}
