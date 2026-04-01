use anyhow::{Context, Result};
use chrono::Utc;

use crate::config::BlueskyConfig;

pub struct CachedSession {
    did: String,
    access_jwt: String,
    created_at: chrono::DateTime<Utc>,
}

impl CachedSession {
    fn is_expired(&self) -> bool {
        Utc::now().signed_duration_since(self.created_at) > chrono::Duration::minutes(90)
    }
}

pub async fn post_with_session(
    config: &BlueskyConfig,
    text: &str,
    cached: &mut Option<CachedSession>,
) -> Result<String> {
    let client = reqwest::Client::new();

    let needs_refresh = cached.as_ref().is_none_or(|s| s.is_expired());
    if needs_refresh {
        let session = create_session(&client, config).await?;
        *cached = Some(session);
    }

    let session = cached.as_ref().unwrap();

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
        if status.as_u16() == 401 {
            *cached = None;
        }
        anyhow::bail!("Bluesky API error {}: {}", status, body);
    }

    let result: serde_json::Value = response.json().await?;
    Ok(result["uri"].as_str().unwrap_or("unknown").to_string())
}

async fn create_session(client: &reqwest::Client, config: &BlueskyConfig) -> Result<CachedSession> {
    let body = serde_json::json!({
        "identifier": config.identifier,
        "password": config.password,
    });

    let response = client
        .post(format!("{}/xrpc/com.atproto.server.createSession", config.service_url))
        .json(&body)
        .send()
        .await
        .context("Failed to create Bluesky session")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("Bluesky auth error {}: {}", status, body);
    }

    let result: serde_json::Value = response.json().await.context("Failed to parse Bluesky session")?;

    let did = result["did"].as_str().context("Missing 'did' in Bluesky session")?.to_string();
    let access_jwt = result["accessJwt"].as_str().context("Missing 'accessJwt' in Bluesky session")?.to_string();

    Ok(CachedSession {
        did,
        access_jwt,
        created_at: Utc::now(),
    })
}
