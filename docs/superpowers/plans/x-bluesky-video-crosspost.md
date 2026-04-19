# X and Bluesky Video Cross-Post Plan

> **For agentic workers:** Use superpowers:subagent-driven-development.

**Goal:** Extend `XPlatform` and `BlueskyPlatform` to attach the chart video when `SocialPost.video_mp4_path` is set. After this PR the same rendered MP4 that TikTok receives is also attached to the X tweet and Bluesky post, giving free reach on platforms we already post to. When `video_mp4_path` is absent, behaviour is unchanged.

**Architecture:** No new modules. Each platform's `publish()` gets a conditional media-upload branch.

**Dependencies:** Gluebox PR #11 merged.

---

## Task 1: Bluesky video embed

Bluesky supports video via `app.bsky.embed.video` + a blob ref from `com.atproto.repo.uploadBlob`. Size cap 50MB / 3min (our clips are ~15MB / 30s).

**File:** `src/connectors/stonkwatch_social/bluesky.rs`

1. Add a helper:

```rust
async fn upload_bluesky_video(
    client: &reqwest::Client,
    config: &BlueskyConfig,
    session: &CachedSession,
    video_path: &std::path::Path,
) -> Result<serde_json::Value> {
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
```

2. In `publish()`, after the existing thumb upload, conditionally upload video. Modify `build_embed` call so when a video blob is present it returns `{"$type": "app.bsky.embed.video", "video": <blob>}` instead of the external-link card (Bluesky embed is an XOR union — video OR external card, not both).

3. `cargo +nightly check` clean. Commit: `feat(bluesky): embed chart video when SocialPost carries video_mp4_path`.

## Task 2: X video embed (chunked upload)

X requires `media_category=tweet_video` via v1.1 chunked upload (INIT → APPEND → FINALIZE → STATUS poll).

**File:** `src/connectors/stonkwatch_social/x.rs`

1. Add `upload_x_video(config, tokens, store_path, video_path) -> Result<String>` that:
   - Reads the MP4 bytes
   - POST to `https://upload.twitter.com/1.1/media/upload.json` with `command=INIT`, `total_bytes=<size>`, `media_type=video/mp4`, `media_category=tweet_video`. Parse `media_id_string`
   - POST APPEND chunks (≤5MB each) with `command=APPEND`, `media_id`, `segment_index`, and the chunk as multipart `media`. For our small clips (~15MB) this is ~3 chunks
   - POST `command=FINALIZE`, `media_id`. Response includes `processing_info.check_after_secs`
   - Poll `command=STATUS` with `media_id` every `check_after_secs` (use a short cap — 30s max total wait) until `processing_info.state == "succeeded"` or fail
   - Return `media_id_string`

2. In `post_tweet(...)`, before building the body:

```rust
let media_ids = match video_path {
    Some(path) => upload_x_video(config, tokens, store_path, path)
        .await
        .map(|id| vec![id])
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "X video upload failed, posting text-only");
            vec![]
        }),
    None => vec![],
};
```

3. Inject into the tweet body JSON when non-empty:

```rust
if !media_ids.is_empty() {
    body["media"] = serde_json::json!({ "media_ids": media_ids });
}
```

4. Commit: `feat(x): upload chart video via chunked media API and attach to tweet`.

## Verification

- `cargo +nightly check` clean
- `cargo +nightly clippy --all-targets -- -D warnings` — no new warnings
- Manual (Bluesky): trigger a trending entity with a real announcement. Verify the Bluesky post shows an inline video.
- Manual (X): same. Verify the tweet has an attached MP4 that plays.
- Regression: post with `video_mp4_path = None` still works on both platforms (text-only with OG image as before).

## Known issues

- **X video upload takes several seconds of STATUS polling** before the tweet can be created. The current `post_to_all_platforms` iterator is sequential with 5s sleeps — video upload may push individual cycles past that. Acceptable for v1; longer-term consider async dispatch.
- **X free tier may reject video uploads** — if the account is on free, `upload_x_video` will fail and the code falls through to text-only. Document in config.
- **Bluesky 50MB / 3min cap** — our VerticalV1 clips are safely under. Recheck if we ever change the `RenderProfile`.
