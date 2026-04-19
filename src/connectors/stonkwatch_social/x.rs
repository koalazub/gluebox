use anyhow::{Context, Result};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::config::XConfig;
use super::platform::{SocialPlatform, SocialPost, PostResult, check_response};

const TOKEN_STORE_PATH: &str = "/var/lib/gluebox/x-tokens.toml";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct StoredTokens {
    access_token: String,
    refresh_token: Option<String>,
}

pub struct XPlatform {
    config: XConfig,
    tokens: Arc<Mutex<StoredTokens>>,
    store_path: PathBuf,
}

impl XPlatform {
    pub fn new(config: XConfig) -> Self {
        let store_path = PathBuf::from(TOKEN_STORE_PATH);
        let tokens = load_stored_tokens(&store_path).unwrap_or_else(|| StoredTokens {
            access_token: config.access_token.clone(),
            refresh_token: config.refresh_token.clone(),
        });
        Self {
            config,
            tokens: Arc::new(Mutex::new(tokens)),
            store_path,
        }
    }
}

fn load_stored_tokens(path: &Path) -> Option<StoredTokens> {
    let content = std::fs::read_to_string(path).ok()?;
    match toml::from_str::<StoredTokens>(&content) {
        Ok(t) => {
            tracing::info!(path = %path.display(), "Loaded X tokens from store");
            Some(t)
        }
        Err(e) => {
            tracing::warn!(%e, "X token store file exists but failed to parse, falling back to config");
            None
        }
    }
}

fn persist_tokens(path: &Path, tokens: &StoredTokens) -> Result<()> {
    let serialized = toml::to_string(tokens).context("failed to serialize X tokens")?;
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, serialized).context("failed to write X token tmp file")?;
    std::fs::rename(&tmp, path).context("failed to rename X token tmp file into place")?;
    Ok(())
}

impl SocialPlatform for XPlatform {
    fn name(&self) -> &'static str {
        "x"
    }

    fn publish<'a>(&'a self, post: &'a SocialPost) -> Pin<Box<dyn Future<Output = Result<PostResult>> + Send + 'a>> {
        Box::pin(async move {
            let id = post_tweet(&self.config, &self.tokens, &self.store_path, &post.text, post.video_mp4_path.as_deref()).await?;
            Ok(PostResult { platform: "x", id })
        })
    }
}

pub async fn post_tweet(
    config: &XConfig,
    tokens: &Mutex<StoredTokens>,
    store_path: &Path,
    text: &str,
    video_path: Option<&Path>,
) -> Result<String> {
    let client = reqwest::Client::new();

    let media_ids = match video_path {
        Some(path) => upload_x_video(tokens, path)
            .await
            .map(|id| vec![id])
            .unwrap_or_else(|e| {
                tracing::warn!(error = %e, "X video upload failed, posting text-only");
                vec![]
            }),
        None => vec![],
    };

    let current_access_token = tokens.lock().await.access_token.clone();
    let response = post_with_token(&client, text, &current_access_token, &media_ids).await?;

    if response.status().as_u16() == 401 {
        let current_refresh = tokens.lock().await.refresh_token.clone();
        let Some(refresh) = current_refresh else {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("X API auth error 401: {} — no refresh token available, re-authorization required", body);
        };

        tracing::warn!(platform = "x", "Access token expired, attempting refresh");
        let new_tokens = refresh_access_token(&client, config, &refresh).await?;

        {
            let mut locked = tokens.lock().await;
            locked.access_token = new_tokens.access_token.clone();
            if new_tokens.refresh_token.is_some() {
                locked.refresh_token = new_tokens.refresh_token.clone();
            }
            if let Err(e) = persist_tokens(store_path, &locked) {
                tracing::warn!(%e, "failed to persist refreshed X tokens — in-memory update only");
            } else {
                tracing::info!(path = %store_path.display(), "persisted refreshed X tokens");
            }
        }

        let retry = post_with_token(&client, text, &new_tokens.access_token, &media_ids).await?;
        return parse_tweet_id(retry).await;
    }

    parse_tweet_id(response).await
}

const CHUNK_SIZE: usize = 5 * 1024 * 1024;

async fn upload_x_video(
    tokens: &Mutex<StoredTokens>,
    video_path: &Path,
) -> Result<String> {
    let client = reqwest::Client::new();
    let bytes = tokio::fs::read(video_path).await.context("read video for X upload")?;
    let total_bytes = bytes.len();

    let access_token = tokens.lock().await.access_token.clone();

    let init_resp = client
        .post("https://upload.twitter.com/1.1/media/upload.json")
        .bearer_auth(&access_token)
        .form(&[
            ("command", "INIT"),
            ("total_bytes", &total_bytes.to_string()),
            ("media_type", "video/mp4"),
            ("media_category", "tweet_video"),
        ])
        .send()
        .await
        .context("X media INIT failed")?;

    let init_resp = check_response(init_resp, "X media INIT").await?;
    let init_json: serde_json::Value = init_resp.json().await.context("X media INIT parse")?;
    let media_id = init_json["media_id_string"]
        .as_str()
        .context("X media INIT missing media_id_string")?
        .to_string();

    for (segment_index, chunk) in bytes.chunks(CHUNK_SIZE).enumerate() {
        let form = reqwest::multipart::Form::new()
            .part("command", reqwest::multipart::Part::text("APPEND"))
            .part("media_id", reqwest::multipart::Part::text(media_id.clone()))
            .part("segment_index", reqwest::multipart::Part::text(segment_index.to_string()))
            .part("media", reqwest::multipart::Part::bytes(chunk.to_vec()));

        let append_resp = client
            .post("https://upload.twitter.com/1.1/media/upload.json")
            .bearer_auth(&access_token)
            .multipart(form)
            .send()
            .await
            .context("X media APPEND failed")?;

        let status = append_resp.status();
        if !status.is_success() {
            let body = append_resp.text().await.unwrap_or_default();
            anyhow::bail!("X media APPEND error {} at segment {}: {}", status, segment_index, body);
        }
    }

    let finalize_resp = client
        .post("https://upload.twitter.com/1.1/media/upload.json")
        .bearer_auth(&access_token)
        .form(&[
            ("command", "FINALIZE"),
            ("media_id", media_id.as_str()),
        ])
        .send()
        .await
        .context("X media FINALIZE failed")?;

    let finalize_resp = check_response(finalize_resp, "X media FINALIZE").await?;
    let finalize_json: serde_json::Value = finalize_resp.json().await.context("X media FINALIZE parse")?;

    if let Some(check_after) = finalize_json["processing_info"]["check_after_secs"].as_u64() {
        poll_x_media_status(&client, &access_token, &media_id, check_after).await?;
    }

    Ok(media_id)
}

async fn poll_x_media_status(
    client: &reqwest::Client,
    access_token: &str,
    media_id: &str,
    initial_check_after: u64,
) -> Result<()> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let mut check_after = initial_check_after;

    loop {
        tokio::time::sleep(std::time::Duration::from_secs(check_after.min(30))).await;

        if std::time::Instant::now() >= deadline {
            anyhow::bail!("X media processing timed out after 30s for media_id {}", media_id);
        }

        let status_resp = client
            .get("https://upload.twitter.com/1.1/media/upload.json")
            .bearer_auth(access_token)
            .query(&[("command", "STATUS"), ("media_id", media_id)])
            .send()
            .await
            .context("X media STATUS failed")?;

        let status_resp = check_response(status_resp, "X media STATUS").await?;
        let status_json: serde_json::Value = status_resp.json().await.context("X media STATUS parse")?;

        let state = status_json["processing_info"]["state"].as_str().unwrap_or("");
        match state {
            "succeeded" => return Ok(()),
            "failed" => {
                let error = &status_json["processing_info"]["error"];
                anyhow::bail!("X media processing failed: {}", error);
            }
            _ => {
                check_after = status_json["processing_info"]["check_after_secs"]
                    .as_u64()
                    .unwrap_or(5);
            }
        }
    }
}

async fn post_with_token(client: &reqwest::Client, text: &str, token: &str, media_ids: &[String]) -> Result<reqwest::Response> {
    let mut body = serde_json::json!({ "text": text });

    if !media_ids.is_empty() {
        body["media"] = serde_json::json!({ "media_ids": media_ids });
    }

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

async fn refresh_access_token(client: &reqwest::Client, config: &XConfig, refresh_token: &str) -> Result<StoredTokens> {
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
    let access_token = result["access_token"]
        .as_str()
        .map(|s| s.to_string())
        .context("Missing 'access_token' in refresh response")?;
    let refresh_token = result["refresh_token"].as_str().map(|s| s.to_string());
    Ok(StoredTokens { access_token, refresh_token })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persist_and_load_roundtrip() {
        let tmp = std::env::temp_dir().join(format!("x-tokens-test-{}.toml", std::process::id()));
        let original = StoredTokens {
            access_token: "atk_123".into(),
            refresh_token: Some("rtk_456".into()),
        };
        persist_tokens(&tmp, &original).unwrap();
        let loaded = load_stored_tokens(&tmp).unwrap();
        assert_eq!(loaded.access_token, "atk_123");
        assert_eq!(loaded.refresh_token, Some("rtk_456".into()));
        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn load_returns_none_when_missing() {
        let missing = std::env::temp_dir().join(format!("x-tokens-missing-{}.toml", std::process::id()));
        assert!(load_stored_tokens(&missing).is_none());
    }
}
