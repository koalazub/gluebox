use anyhow::{Context, Result};
use chrono::Utc;

use crate::config::BlueskyConfig;
use super::content::PostCandidate;

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
    post: &PostCandidate,
    cached: &mut Option<CachedSession>,
) -> Result<String> {
    let client = reqwest::Client::new();

    if cached.as_ref().is_none_or(|s| s.is_expired()) {
        *cached = Some(create_session(&client, config).await?);
    }
    let session = cached.as_ref().unwrap();

    let facets = build_facets(&post.text, &post.link);

    let thumb = upload_og_image(&client, config, session, post.og_image_path.as_deref()).await;

    let embed = build_embed(post, thumb);

    let mut record = serde_json::json!({
        "$type": "app.bsky.feed.post",
        "text": post.text,
        "createdAt": Utc::now().to_rfc3339(),
        "langs": ["en"],
        "facets": facets,
        "embed": embed,
    });

    if facets.is_empty() {
        record.as_object_mut().unwrap().remove("facets");
    }

    let body = serde_json::json!({
        "repo": session.did,
        "collection": "app.bsky.feed.post",
        "record": record,
    });

    let response = client
        .post(format!("{}/xrpc/com.atproto.repo.createRecord", config.service_url))
        .header("Authorization", format!("Bearer {}", session.access_jwt))
        .json(&body)
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
    result["uri"]
        .as_str()
        .map(|s| s.to_string())
        .context("Missing 'uri' in Bluesky response")
}

fn build_facets(text: &str, link: &str) -> Vec<serde_json::Value> {
    let mut facets = Vec::new();
    if let Some(byte_start) = text.find(link) {
        facets.push(serde_json::json!({
            "index": {
                "byteStart": byte_start,
                "byteEnd": byte_start + link.len(),
            },
            "features": [{
                "$type": "app.bsky.richtext.facet#link",
                "uri": link,
            }]
        }));
    }
    facets
}

fn build_embed(post: &PostCandidate, thumb: Option<serde_json::Value>) -> serde_json::Value {
    let mut external = serde_json::json!({
        "uri": post.link,
        "title": post.og_title(),
        "description": post.og_description(),
    });

    if let Some(blob) = thumb {
        external["thumb"] = blob;
    }

    serde_json::json!({
        "$type": "app.bsky.embed.external",
        "external": external,
    })
}

async fn upload_og_image(
    client: &reqwest::Client,
    config: &BlueskyConfig,
    session: &CachedSession,
    image_path: Option<&str>,
) -> Option<serde_json::Value> {
    let path = image_path?;
    let bytes = tokio::fs::read(path).await.ok()?;

    if bytes.len() > 1_000_000 {
        tracing::warn!(path, "OG image too large for Bluesky, skipping");
        return None;
    }

    let response = client
        .post(format!("{}/xrpc/com.atproto.repo.uploadBlob", config.service_url))
        .header("Authorization", format!("Bearer {}", session.access_jwt))
        .header("Content-Type", "image/png")
        .body(bytes)
        .send()
        .await
        .ok()?;

    if !response.status().is_success() {
        tracing::warn!(path, status = %response.status(), "Bluesky image upload failed");
        return None;
    }

    let result: serde_json::Value = response.json().await.ok()?;
    Some(result["blob"].clone())
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

    Ok(CachedSession {
        did: result["did"].as_str().context("Missing 'did'")?.to_string(),
        access_jwt: result["accessJwt"].as_str().context("Missing 'accessJwt'")?.to_string(),
        created_at: Utc::now(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_candidate(text: &str, link: &str) -> PostCandidate {
        PostCandidate {
            announcement_id: "test-123".into(),
            symbol: "BHP".into(),
            title: "Iron Ore Quarterly Update".into(),
            ann_type: "price_sensitive".into(),
            importance: "high".into(),
            summary: Some("BHP reported record iron ore output, beating estimates by 12%".into()),
            link: link.into(),
            text: text.into(),
            og_image_path: None,
        }
    }

    #[test]
    fn facets_mark_link_position() {
        let link = "https://stonkwatch.app/announcement/test-123?utm_source=social&utm_medium=bot";
        let text = format!("$BHP iron ore output up 12%. {}", link);
        let facets = build_facets(&text, link);

        assert_eq!(facets.len(), 1);
        let start = facets[0]["index"]["byteStart"].as_u64().unwrap() as usize;
        let end = facets[0]["index"]["byteEnd"].as_u64().unwrap() as usize;
        assert_eq!(&text[start..end], link);
    }

    #[test]
    fn facets_empty_when_link_not_in_text() {
        let facets = build_facets("no link here", "https://example.com");
        assert!(facets.is_empty());
    }

    #[test]
    fn embed_includes_structured_og_data() {
        let link = "https://stonkwatch.app/announcement/test-123";
        let post = test_candidate("test", link);
        let embed = build_embed(&post, None);

        assert_eq!(embed["$type"], "app.bsky.embed.external");
        assert_eq!(embed["external"]["uri"], link);
        assert_eq!(embed["external"]["title"], "$BHP ⚡ — Iron Ore Quarterly Update");
        assert!(embed["external"]["description"].as_str().unwrap().contains("record iron ore output"));
        assert!(embed["external"]["thumb"].is_null());
    }

    #[test]
    fn embed_includes_thumb_when_provided() {
        let post = test_candidate("test", "https://example.com");
        let thumb = serde_json::json!({"$type": "blob", "ref": {"$link": "abc123"}, "mimeType": "image/png", "size": 1234});
        let embed = build_embed(&post, Some(thumb.clone()));

        assert_eq!(embed["external"]["thumb"], thumb);
    }

    #[test]
    fn post_candidate_og_title_with_price_sensitive() {
        let post = test_candidate("test", "https://example.com");
        assert_eq!(post.og_title(), "$BHP ⚡ — Iron Ore Quarterly Update");
    }

    #[test]
    fn post_candidate_og_title_without_price_sensitive() {
        let mut post = test_candidate("test", "https://example.com");
        post.ann_type = "general".into();
        post.importance = "low".into();
        assert_eq!(post.og_title(), "$BHP — Iron Ore Quarterly Update");
    }

    #[test]
    fn post_candidate_og_description_truncates_long_summary() {
        let mut post = test_candidate("test", "https://example.com");
        post.summary = Some("x".repeat(300));
        let desc = post.og_description();
        assert!(desc.len() <= 203);
        assert!(desc.ends_with("..."));
    }

    #[test]
    fn post_candidate_og_description_fallback_without_summary() {
        let mut post = test_candidate("test", "https://example.com");
        post.summary = None;
        let desc = post.og_description();
        assert!(desc.contains("BHP"));
        assert!(desc.contains("ASX"));
    }
}
