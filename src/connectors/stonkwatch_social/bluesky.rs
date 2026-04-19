use anyhow::{Context, Result};
use chrono::Utc;
use std::future::Future;
use std::pin::Pin;

use crate::config::BlueskyConfig;
use super::platform::{SocialPlatform, SocialPost, PostResult, check_response};

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

pub struct BlueskyPlatform {
    config: BlueskyConfig,
    session: tokio::sync::Mutex<Option<CachedSession>>,
}

impl BlueskyPlatform {
    pub fn new(config: BlueskyConfig) -> Self {
        Self {
            config,
            session: tokio::sync::Mutex::new(None),
        }
    }
}

impl SocialPlatform for BlueskyPlatform {
    fn name(&self) -> &'static str {
        "bluesky"
    }

    fn publish<'a>(&'a self, post: &'a SocialPost) -> Pin<Box<dyn Future<Output = Result<PostResult>> + Send + 'a>> {
        Box::pin(async move {
            let client = reqwest::Client::new();
            let mut session_guard = self.session.lock().await;

            if session_guard.as_ref().is_none_or(|s| s.is_expired()) {
                *session_guard = Some(create_session(&client, &self.config).await?);
            }
            let session = session_guard.as_ref().unwrap();

            let facets = build_facets(&post.text, &post.link);

            let video_blob = if let Some(ref video_path) = post.video_mp4_path {
                match upload_bluesky_video(&client, &self.config, session, video_path).await {
                    Ok(blob) => Some(blob),
                    Err(e) => {
                        tracing::warn!(error = %e, "Bluesky video upload failed, falling back to link card");
                        None
                    }
                }
            } else {
                None
            };

            let thumb = if video_blob.is_none() {
                upload_og_image(&client, &self.config, session, post.image_url.as_deref()).await
            } else {
                None
            };

            let embed = build_video_or_link_embed(video_blob, &post.link, &post.og_title, &post.og_description, thumb);

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
                .post(format!("{}/xrpc/com.atproto.repo.createRecord", self.config.service_url))
                .header("Authorization", format!("Bearer {}", session.access_jwt))
                .json(&body)
                .send()
                .await
                .context("Failed to create Bluesky post")?;

            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                if status.as_u16() == 401 {
                    *session_guard = None;
                }
                anyhow::bail!("Bluesky API error {}: {}", status, body);
            }

            let result: serde_json::Value = response.json().await?;
            let uri = result["uri"]
                .as_str()
                .map(|s| s.to_string())
                .context("Missing 'uri' in Bluesky response")?;

            Ok(PostResult { platform: "bluesky", id: uri })
        })
    }
}

fn build_video_or_link_embed(
    video_blob: Option<serde_json::Value>,
    link: &str,
    og_title: &str,
    og_description: &str,
    thumb: Option<serde_json::Value>,
) -> serde_json::Value {
    if let Some(blob) = video_blob {
        serde_json::json!({
            "$type": "app.bsky.embed.video",
            "video": blob,
        })
    } else {
        build_embed(link, og_title, og_description, thumb)
    }
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

fn build_embed(link: &str, og_title: &str, og_description: &str, thumb: Option<serde_json::Value>) -> serde_json::Value {
    let mut external = serde_json::json!({
        "uri": link,
        "title": og_title,
        "description": og_description,
    });

    if let Some(blob) = thumb {
        external["thumb"] = blob;
    }

    serde_json::json!({
        "$type": "app.bsky.embed.external",
        "external": external,
    })
}

async fn upload_bluesky_video(
    client: &reqwest::Client,
    config: &BlueskyConfig,
    session: &CachedSession,
    video_path: &std::path::Path,
) -> Result<serde_json::Value> {
    const BLUESKY_VIDEO_MAX: u64 = 50 * 1024 * 1024;
    const BLUESKY_VIDEO_MAX_LABEL: &str = "50 MiB";
    let meta = tokio::fs::metadata(video_path).await.context("stat video for bluesky")?;
    if meta.len() > BLUESKY_VIDEO_MAX {
        anyhow::bail!(
            "bluesky video rejected: {} bytes > {} limit",
            meta.len(),
            BLUESKY_VIDEO_MAX_LABEL
        );
    }

    let bytes = tokio::fs::read(video_path).await.context("read video")?;
    let resp = client
        .post(format!("{}/xrpc/com.atproto.repo.uploadBlob", config.service_url))
        .bearer_auth(&session.access_jwt)
        .header("Content-Type", "video/mp4")
        .body(bytes)
        .send()
        .await
        .context("bluesky uploadBlob failed")?;
    let resp = check_response(resp, "bluesky-uploadBlob").await?;
    let v: serde_json::Value = resp.json().await.context("bluesky uploadBlob parse")?;
    v.get("blob").cloned().ok_or_else(|| anyhow::anyhow!("bluesky uploadBlob missing 'blob'"))
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

    let response = check_response(response, "Bluesky auth").await?;
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

    fn test_social_post(text: &str, link: &str) -> SocialPost {
        SocialPost {
            text: text.into(),
            link: link.into(),
            image_url: None,
            story_image_url: None,
            og_title: "$BHP ⚡ — Iron Ore Quarterly Update".into(),
            og_description: "BHP reported record iron ore output, beating estimates by 12%".into(),
            video_mp4_path: None,
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
        let post = test_social_post("test", link);
        let embed = build_embed(&post.link, &post.og_title, &post.og_description, None);

        assert_eq!(embed["$type"], "app.bsky.embed.external");
        assert_eq!(embed["external"]["uri"], link);
        assert_eq!(embed["external"]["title"], "$BHP ⚡ — Iron Ore Quarterly Update");
        assert!(embed["external"]["description"].as_str().unwrap().contains("record iron ore output"));
        assert!(embed["external"]["thumb"].is_null());
    }

    #[test]
    fn embed_includes_thumb_when_provided() {
        let post = test_social_post("test", "https://example.com");
        let thumb = serde_json::json!({"$type": "blob", "ref": {"$link": "abc123"}, "mimeType": "image/png", "size": 1234});
        let embed = build_embed(&post.link, &post.og_title, &post.og_description, Some(thumb.clone()));

        assert_eq!(embed["external"]["thumb"], thumb);
    }

    #[test]
    fn video_embed_selected_when_blob_present() {
        let blob = serde_json::json!({"$type": "blob", "ref": {"$link": "vid123"}, "mimeType": "video/mp4", "size": 5000000});
        let embed = build_video_or_link_embed(
            Some(blob.clone()),
            "https://stonkwatch.app/announcement/test",
            "Test Title",
            "Test description",
            None,
        );
        assert_eq!(embed["$type"], "app.bsky.embed.video");
        assert_eq!(embed["video"], blob);
    }

    #[test]
    fn link_card_embed_selected_when_no_blob() {
        let embed = build_video_or_link_embed(
            None,
            "https://stonkwatch.app/announcement/test",
            "Test Title",
            "Test description",
            None,
        );
        assert_eq!(embed["$type"], "app.bsky.embed.external");
        assert_eq!(embed["external"]["uri"], "https://stonkwatch.app/announcement/test");
        assert_eq!(embed["external"]["title"], "Test Title");
    }
}
