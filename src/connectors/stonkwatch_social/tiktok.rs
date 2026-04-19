use anyhow::{Context, Result};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::config::TikTokConfig;
use super::platform::{SocialPlatform, SocialPost, PostResult, check_response};

const TOKEN_STORE_PATH: &str = "/var/lib/gluebox/tiktok-tokens.toml";
const TIKTOK_API_BASE: &str = "https://open.tiktokapis.com/v2";
const TIKTOK_TITLE_MAX: usize = 150;
const TIKTOK_VIDEO_MAX_BYTES: usize = 4 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct StoredTokens {
    access_token: String,
    refresh_token: Option<String>,
}

pub struct TikTokPlatform {
    config: TikTokConfig,
    tokens: Arc<Mutex<StoredTokens>>,
    store_path: PathBuf,
}

impl TikTokPlatform {
    pub fn new(config: TikTokConfig) -> Self {
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
    toml::from_str(&content).ok()
}

fn persist_tokens(path: &Path, tokens: &StoredTokens) -> Result<()> {
    let serialized = toml::to_string(tokens).context("serialize tiktok tokens")?;
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, serialized).context("write tiktok tokens tmp")?;
    std::fs::rename(&tmp, path).context("rename tiktok tokens into place")?;
    Ok(())
}

impl SocialPlatform for TikTokPlatform {
    fn name(&self) -> &'static str {
        "tiktok"
    }

    fn accepts(&self, post: &SocialPost) -> bool {
        post.video_mp4_path.is_some()
    }

    fn publish<'a>(
        &'a self,
        post: &'a SocialPost,
    ) -> Pin<Box<dyn Future<Output = Result<PostResult>> + Send + 'a>> {
        Box::pin(async move {
            let video_path = post
                .video_mp4_path
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("TikTok requires a video_mp4_path; none provided"))?;

            let publish_id = upload_video(
                &self.config,
                &self.tokens,
                &self.store_path,
                video_path,
                tiktok_title(&post.text),
            )
            .await?;
            Ok(PostResult {
                platform: "tiktok",
                id: publish_id,
            })
        })
    }
}

fn tiktok_title(text: &str) -> String {
    if text.chars().count() <= TIKTOK_TITLE_MAX {
        text.to_string()
    } else {
        let cut: String = text.chars().take(TIKTOK_TITLE_MAX - 1).collect();
        format!("{cut}…")
    }
}

#[derive(serde::Deserialize)]
struct InitData {
    publish_id: String,
    upload_url: String,
}

#[derive(serde::Deserialize)]
struct InitResponse {
    data: InitData,
}

pub async fn upload_video(
    config: &TikTokConfig,
    tokens: &Mutex<StoredTokens>,
    store_path: &Path,
    video_path: &Path,
    title: String,
) -> Result<String> {
    let bytes = tokio::fs::read(video_path).await.context("read video file")?;
    let size = bytes.len();
    if size == 0 {
        anyhow::bail!("tiktok upload: video file {} is empty", video_path.display());
    }
    if size > TIKTOK_VIDEO_MAX_BYTES {
        anyhow::bail!(
            "tiktok upload refused: video file {} is {} bytes (> {}B limit)",
            video_path.display(),
            size,
            TIKTOK_VIDEO_MAX_BYTES
        );
    }

    let access_token = { tokens.lock().await.access_token.clone() };

    let init_body = serde_json::json!({
        "post_info": {
            "title": title,
            "privacy_level": config.privacy_level,
            "disable_duet": config.disable_duet,
            "disable_stitch": config.disable_stitch,
            "disable_comment": config.disable_comment,
        },
        "source_info": {
            "source": "FILE_UPLOAD",
            "video_size": size,
            "chunk_size": size,
            "total_chunk_count": 1,
        }
    });

    let client = reqwest::Client::new();
    let init_url = format!("{TIKTOK_API_BASE}/post/publish/video/init/");

    let mut init_resp = client
        .post(&init_url)
        .bearer_auth(&access_token)
        .header("Content-Type", "application/json")
        .json(&init_body)
        .send()
        .await
        .context("tiktok init request failed")?;

    if init_resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        refresh_access_token(config, tokens, store_path).await?;
        let retry_token = { tokens.lock().await.access_token.clone() };
        init_resp = client
            .post(&init_url)
            .bearer_auth(&retry_token)
            .header("Content-Type", "application/json")
            .json(&init_body)
            .send()
            .await
            .context("tiktok init retry after refresh failed")?;
    }

    let init_resp = check_response(init_resp, "tiktok").await?;
    let parsed: InitResponse = init_resp
        .json()
        .await
        .context("tiktok init response parse")?;
    let publish_id = parsed.data.publish_id;
    let upload_url = parsed.data.upload_url;

    let upload_resp = client
        .put(&upload_url)
        .header(
            "Content-Range",
            format!("bytes 0-{}/{}", size.saturating_sub(1), size),
        )
        .header("Content-Type", "video/mp4")
        .body(bytes)
        .send()
        .await
        .context("tiktok video PUT failed")?;

    if !upload_resp.status().is_success() {
        let status = upload_resp.status();
        let text = upload_resp.text().await.unwrap_or_default();
        anyhow::bail!("tiktok video PUT returned {}: {}", status, text);
    }

    Ok(publish_id)
}

#[derive(serde::Deserialize)]
struct RefreshResponse {
    access_token: String,
    refresh_token: Option<String>,
}

async fn refresh_access_token(
    config: &TikTokConfig,
    tokens: &Mutex<StoredTokens>,
    store_path: &Path,
) -> Result<()> {
    let refresh_token = {
        tokens
            .lock()
            .await
            .refresh_token
            .clone()
            .ok_or_else(|| anyhow::anyhow!("no refresh_token stored for tiktok"))?
    };

    let client = reqwest::Client::new();
    let params = [
        ("client_key", config.client_key.as_str()),
        ("client_secret", config.client_secret.as_str()),
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token.as_str()),
    ];

    let resp = client
        .post("https://open.tiktokapis.com/v2/oauth/token/")
        .form(&params)
        .send()
        .await
        .context("tiktok refresh request failed")?;

    let resp = check_response(resp, "tiktok-refresh").await?;
    let refreshed: RefreshResponse = resp
        .json()
        .await
        .context("tiktok refresh response parse")?;

    let mut guard = tokens.lock().await;
    guard.access_token = refreshed.access_token;
    if let Some(rt) = refreshed.refresh_token {
        guard.refresh_token = Some(rt);
    }
    if let Err(e) = persist_tokens(store_path, &guard) {
        tracing::warn!(%e, path = %store_path.display(), "tiktok token persist failed; keeping refreshed tokens in memory");
    }
    Ok(())
}
