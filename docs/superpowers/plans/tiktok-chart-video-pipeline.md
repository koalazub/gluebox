# Gluebox TikTok Chart-Video Pipeline Implementation Plan

> **For agentic workers:** Use superpowers:subagent-driven-development. Steps use `- [ ]` for tracking.

**Goal:** Wire gluebox's `stonkwatch_social` pipeline to render chart videos via the new stonkwatch `POST /api/v1/chart-video/announcement` endpoint (built in stonkwatch PR #213) and post them to TikTok. This supersedes `docs/TIKTOK_CONNECTOR_SPEC.md` (which proposed local Ken Burns on OG cards — replaced by server-rendered animated price reveals).

**Architecture:** Gluebox calls stonkwatch's HTTP endpoint to get an MP4, uploads it to TikTok via the Content Posting API. Existing `trending_to_social.rs` trigger and `StonkwatchSocialConnector::build_platforms()` are extended — no new trigger or orchestrator.

**Tech stack:** Rust (nightly via nix), tokio, reqwest, axum, turso (via existing `AppState.db`), `jj` for VCS.

**Scope (this PR):** Daily reactive TikTok posting (Mon-Thu cadence). Quota + cooldown already enforced by existing `trending_to_social.rs`.

**Out of scope (follow-up PR):** Friday top-5 montage trigger, local ffmpeg stitcher, weekly-post exclude-repeats logic against a new table. All deferred.

---

## File structure

**New:**
- `src/connectors/stonkwatch_social/tiktok.rs` — TikTokPlatform OAuth + Upload API client
- `src/connectors/stonkwatch_social/chart_video.rs` — HTTP client that calls stonkwatch `POST /api/v1/chart-video/announcement`, downloads MP4 bytes

**Modified:**
- `src/config.rs` — add `TikTokConfig`, add `chart_video_api_base: String` to `StonkwatchSocialConfig`
- `src/connectors/stonkwatch_social/platform.rs` — extend `SocialPost` with `video_mp4_path: Option<PathBuf>`
- `src/connectors/stonkwatch_social/pipeline.rs` — `build_candidate` optionally fetches the chart video MP4
- `src/connectors/stonkwatch_social/content.rs` — `PostCandidate.video_mp4_path` field
- `src/connectors/stonkwatch_social/mod.rs` — register `TikTokPlatform` in `build_platforms()`
- `src/triggers/trending_to_social.rs` — pass through video path to candidate → post
- `gluebox.example.toml` — document `[stonkwatch_social.tiktok]` and `chart_video_api_base`

**Unchanged (explicitly):**
- `docs/TIKTOK_CONNECTOR_SPEC.md` — leave in place for now; will mark superseded at end of plan
- `og_image.rs`, `storj.rs` — existing OG image path untouched (X / Bluesky still post the card)
- Daily quota + per-ticker cooldown logic in `trending_to_social.rs` — already correct

---

## Task 1: Config — `TikTokConfig` + `chart_video_api_base`

**Files:** `src/config.rs`, `gluebox.example.toml`

- [ ] Read the existing `StonkwatchSocialConfig` struct in `src/config.rs`. Find the `x: Option<XConfig>`, `bluesky: Option<BlueskyConfig>`, `meta: Option<MetaConfig>` fields.

- [ ] Add `chart_video_api_base: Option<String>` (default None; when None, chart-video rendering is skipped and TikTok posts are dropped).

- [ ] Add `tiktok: Option<TikTokConfig>` alongside the other platform fields.

- [ ] Add the `TikTokConfig` struct:

```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct TikTokConfig {
    pub client_key: String,
    pub client_secret: String,
    pub access_token: String,
    pub refresh_token: Option<String>,
    #[serde(default = "default_tiktok_privacy_level")]
    pub privacy_level: String,
    #[serde(default)]
    pub disable_duet: bool,
    #[serde(default)]
    pub disable_stitch: bool,
    #[serde(default)]
    pub disable_comment: bool,
}

fn default_tiktok_privacy_level() -> String {
    "SELF_ONLY".to_string()
}
```

Note: `SELF_ONLY` puts the video in the inbox (draft), matching the v1 "human-reviews-before-publish" decision.

- [ ] Append to `gluebox.example.toml`:

```toml
[stonkwatch_social]
# existing fields...
chart_video_api_base = "https://api.stonkwatch.au"

[stonkwatch_social.tiktok]
client_key = "your-tiktok-client-key"
client_secret = "your-tiktok-client-secret"
access_token = "your-access-token"
refresh_token = "your-refresh-token"
privacy_level = "SELF_ONLY"
disable_duet = false
disable_stitch = false
disable_comment = false
```

- [ ] `cargo +nightly check` clean.
- [ ] Commit: `jj describe -m "feat(config): add TikTokConfig + chart_video_api_base to stonkwatch_social"` then `jj new`.

---

## Task 2: Chart-video HTTP client

**Files:** `src/connectors/stonkwatch_social/chart_video.rs` (new), `src/connectors/stonkwatch_social/mod.rs`

- [ ] Create `src/connectors/stonkwatch_social/chart_video.rs`:

```rust
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize)]
struct GenerateAnnouncementVideoRequest<'a> {
    symbol: &'a str,
    announcement_id: &'a str,
    window_hours: u32,
    profile: &'a str,
}

#[derive(Debug, Deserialize)]
struct GenerateVideoResponse {
    video_url: String,
    #[allow(dead_code)]
    thumbnail_url: String,
    #[allow(dead_code)]
    duration_secs: u32,
    #[allow(dead_code)]
    symbol: String,
    #[allow(dead_code)]
    profile: String,
}

pub async fn render_announcement_video(
    api_base: &str,
    api_key: &str,
    symbol: &str,
    announcement_id: &str,
    output_dir: &Path,
) -> Result<PathBuf> {
    let url = format!("{}/api/v1/chart-video/announcement", api_base.trim_end_matches('/'));
    let body = GenerateAnnouncementVideoRequest {
        symbol,
        announcement_id,
        window_hours: 6,
        profile: "VerticalV1",
    };

    let client = reqwest::Client::new();
    let resp = client
        .post(&url)
        .header("X-API-Key", api_key)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .context("chart-video render request failed")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("chart-video render returned {}: {}", status, text);
    }

    let parsed: GenerateVideoResponse = resp
        .json()
        .await
        .context("chart-video render returned malformed JSON")?;

    tokio::fs::create_dir_all(output_dir).await.ok();
    let out_path = output_dir.join(format!(
        "{}-{}.mp4",
        symbol,
        chrono::Utc::now().timestamp()
    ));

    let mp4_bytes = client
        .get(&parsed.video_url)
        .send()
        .await
        .context("video mp4 download failed")?
        .error_for_status()
        .context("video mp4 download non-2xx")?
        .bytes()
        .await
        .context("video mp4 body read failed")?;

    tokio::fs::write(&out_path, &mp4_bytes)
        .await
        .context("video mp4 persist failed")?;

    Ok(out_path)
}
```

Note: **the `X-API-Key` header value** is the same stonkwatch API key used elsewhere in gluebox. Check if `StonkwatchSocialConfig` already has an `api_key` or similar field — if not, add `stonkwatch_api_key: Option<String>` to the config (Task 1 footprint).

- [ ] Register module in `src/connectors/stonkwatch_social/mod.rs`:

```rust
pub mod chart_video;
```

- [ ] `cargo +nightly check` clean.
- [ ] Commit: `jj describe -m "feat(chart-video): HTTP client for stonkwatch chart-video render API"` + `jj new`.

---

## Task 3: Thread video path through SocialPost + PostCandidate

**Files:** `src/connectors/stonkwatch_social/platform.rs`, `src/connectors/stonkwatch_social/content.rs`

- [ ] In `platform.rs`, extend `SocialPost`:

```rust
pub struct SocialPost {
    pub text: String,
    pub link: String,
    pub image_url: Option<String>,
    pub story_image_url: Option<String>,
    pub og_title: String,
    pub og_description: String,
    pub video_mp4_path: Option<std::path::PathBuf>,
}
```

- [ ] In `content.rs`, extend `PostCandidate` with `video_mp4_path: Option<PathBuf>`. Extend the `to_social_post` method to forward the field.

- [ ] Update all existing construction sites of `SocialPost` and `PostCandidate` to set `video_mp4_path: None` by default. Grep: `rg "SocialPost \{" src/` and `rg "PostCandidate \{" src/`.

- [ ] `cargo +nightly check` clean — all existing callers still compile with the new field defaulting to `None`.

- [ ] Commit: `jj describe -m "feat(social): thread optional video_mp4_path through SocialPost/PostCandidate"` + `jj new`.

---

## Task 4: TikTok platform — OAuth + Upload API

**Files:** `src/connectors/stonkwatch_social/tiktok.rs` (new), `src/connectors/stonkwatch_social/mod.rs`

- [ ] Read `src/connectors/stonkwatch_social/x.rs` lines 1-100 to crib the token-store pattern (`StoredTokens`, `load_stored_tokens`, `persist_tokens`). Mirror it for TikTok.

- [ ] Create `src/connectors/stonkwatch_social/tiktok.rs`:

```rust
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

    fn publish<'a>(&'a self, post: &'a SocialPost) -> Pin<Box<dyn Future<Output = Result<PostResult>> + Send + 'a>> {
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
            Ok(PostResult { platform: "tiktok", id: publish_id })
        })
    }
}

fn tiktok_title(text: &str) -> String {
    const MAX: usize = 150;
    if text.chars().count() <= MAX {
        text.to_string()
    } else {
        let cut: String = text.chars().take(MAX - 1).collect();
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
    let init_url = format!("{}/post/publish/video/init/", TIKTOK_API_BASE);

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
        let access_token = { tokens.lock().await.access_token.clone() };
        init_resp = client
            .post(&init_url)
            .bearer_auth(&access_token)
            .header("Content-Type", "application/json")
            .json(&init_body)
            .send()
            .await
            .context("tiktok init retry after refresh failed")?;
    }

    let init_resp = check_response(init_resp, "tiktok").await?;
    let parsed: InitResponse = init_resp.json().await.context("tiktok init response parse")?;
    let publish_id = parsed.data.publish_id;
    let upload_url = parsed.data.upload_url;

    let upload_resp = client
        .put(&upload_url)
        .header("Content-Range", format!("bytes 0-{}/{}", size.saturating_sub(1), size))
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
    let refreshed: RefreshResponse = resp.json().await.context("tiktok refresh response parse")?;

    let mut guard = tokens.lock().await;
    guard.access_token = refreshed.access_token;
    if let Some(rt) = refreshed.refresh_token {
        guard.refresh_token = Some(rt);
    }
    persist_tokens(store_path, &*guard)?;
    Ok(())
}
```

- [ ] Register module in `src/connectors/stonkwatch_social/mod.rs`:

```rust
pub mod tiktok;
```

- [ ] In `build_platforms()` (same file), add after the `meta` block:

```rust
if let Some(ref tiktok_cfg) = config.tiktok {
    platforms.push(Box::new(tiktok::TikTokPlatform::new(tiktok_cfg.clone())));
}
```

- [ ] `cargo +nightly check` clean. If clippy flags unused `#[allow(dead_code)]` — these are deliberate for struct fields the compiler can't see as used (they come in through serde). Keep them.

- [ ] Commit: `jj describe -m "feat(tiktok): platform impl with OAuth refresh + Upload API integration"` + `jj new`.

---

## Task 5: Render video in pipeline + plumb to candidate

**Files:** `src/connectors/stonkwatch_social/pipeline.rs`, `src/triggers/trending_to_social.rs`

- [ ] Read `pipeline.rs` lines 1-60 to understand `AnnouncementData`, `prepare_image`, `prepare_story_image`. Note the existing `output_dir` pattern.

- [ ] Add a `prepare_chart_video` fn to `pipeline.rs`:

```rust
pub async fn prepare_chart_video(
    ann: &AnnouncementData,
    output_dir: &std::path::Path,
    api_base: Option<&str>,
    api_key: Option<&str>,
) -> Option<std::path::PathBuf> {
    let api_base = api_base?;
    let api_key = api_key?;
    match super::chart_video::render_announcement_video(
        api_base,
        api_key,
        &ann.symbol,
        &ann.id,
        output_dir,
    )
    .await
    {
        Ok(path) => Some(path),
        Err(e) => {
            tracing::warn!(ticker = %ann.symbol, error = %e, "chart-video render failed");
            None
        }
    }
}
```

- [ ] In `src/triggers/trending_to_social.rs` inside `build_candidate`, after `story_image_path` is set, add:

```rust
let video_mp4_path = pipeline::prepare_chart_video(
    &ann,
    output_dir,
    social_cfg.chart_video_api_base.as_deref(),
    social_cfg.stonkwatch_api_key.as_deref(),
)
.await;
```

(Note: `stonkwatch_api_key` on `StonkwatchSocialConfig` must exist; check Task 1's config changes — if not present, add it there.)

Extend the `PostCandidate { ... }` literal to include:

```rust
video_mp4_path,
```

- [ ] In `content.rs`'s `PostCandidate::to_social_post()`, forward:

```rust
video_mp4_path: self.video_mp4_path.clone(),
```

- [ ] `cargo +nightly check` clean.

- [ ] Commit: `jj describe -m "feat(pipeline): render chart video per announcement and attach to candidate"` + `jj new`.

---

## Task 6: Mark old TikTok spec superseded

**Files:** `docs/TIKTOK_CONNECTOR_SPEC.md`

- [ ] At the top of the file, insert a banner:

```markdown
> **⚠️ SUPERSEDED 2026-04-19.** This spec proposed local Ken Burns zoom over OG cards. The shipped implementation instead calls stonkwatch's server-rendered chart-video API (animated price line reveal), uploaded to TikTok as-is. See `docs/superpowers/plans/2026-04-19-tiktok-chart-video-pipeline.md` for the current plan.
```

- [ ] Commit: `jj describe -m "docs(tiktok): mark local-Ken-Burns spec superseded by server-rendered approach"` + `jj new`.

---

## Verification

Before shipping:

- [ ] `cargo +nightly check` across the workspace — clean
- [ ] `cargo +nightly clippy --all-targets -- -D warnings` — clean
- [ ] `cargo +nightly test` — no regressions (existing tests still green)
- [ ] Smoke: run gluebox with a test `[stonkwatch_social.tiktok]` config pointed at a sandbox TikTok account; verify the inbox receives the video
- [ ] Post-merge manual: trigger a `trending_entity` with `entity_type=stock`, confirm:
  - Chart video renders via stonkwatch HTTP
  - MP4 lands at the configured output_dir
  - TikTok inbox receives the video
  - `trending_posts` row is written

## Follow-ups (separate PR)

1. `src/triggers/friday_digest.rs` — cron Fri 17:00 AEDT, top-5 excluding this-week's posted, render 5 segments via `POST /api/v1/chart-video/segment`, stitch locally via ffmpeg, post to TikTok
2. `src/stitcher.rs` — ffmpeg concat demuxer + transition-card overlays
3. Matrix rollup (5-minute window) for render/upload failures
4. Video attachment for X and Bluesky platforms (they support native video; reuses `video_mp4_path` from `SocialPost`)
