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
            let id = post_tweet(&self.config, &self.tokens, &self.store_path, &post.text).await?;
            Ok(PostResult { platform: "x", id })
        })
    }
}

pub async fn post_tweet(
    config: &XConfig,
    tokens: &Mutex<StoredTokens>,
    store_path: &Path,
    text: &str,
) -> Result<String> {
    let client = reqwest::Client::new();

    let current_access_token = tokens.lock().await.access_token.clone();
    let response = post_with_token(&client, text, &current_access_token).await?;

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

        let retry = post_with_token(&client, text, &new_tokens.access_token).await?;
        return parse_tweet_id(retry).await;
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
