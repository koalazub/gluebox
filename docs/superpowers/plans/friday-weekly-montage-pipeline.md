# Friday Weekly Montage Pipeline Implementation Plan

> **For agentic workers:** Use superpowers:subagent-driven-development.

**Goal:** Add the Friday top-5 weekly retrospective TikTok post deferred from `tiktok-chart-video-pipeline.md`. Each Friday at 17:00 AEDT, fetch the week's top-5 trending tickers (excluding those already posted this week), render an 8-second segment for each via stonkwatch's `POST /api/v1/chart-video/segment`, stitch with ffmpeg, upload to TikTok.

**Architecture:** New cron-driven trigger alongside `trending_to_social.rs`. Stonkwatch renders segments server-side; gluebox stitches locally (ffmpeg on PATH). Posts go through the existing `TikTokPlatform`.

**Dependencies:** Merged `tiktok-chart-video-pipeline.md` (TikTok platform + chart-video HTTP client). ffmpeg in `flake.nix` runtime env.

---

## New files

- `src/triggers/friday_digest.rs` — cron loop, top-5 selection, exclude-repeats, stitch-and-post
- `src/connectors/stonkwatch_social/stitcher.rs` — ffmpeg concat demuxer wrapper

## Modified files

- `src/connectors/stonkwatch_social/chart_video.rs` — add `render_segment(api_base, api_key, symbol, announcement_id: Option<&str>, window_days, duration_secs, output_dir) -> Result<PathBuf>` calling `POST /api/v1/chart-video/segment`
- `src/db.rs` — add `tickers_posted_this_iso_week()` + `weekly_digest_posted_this_iso_week()` helpers for exclude-repeats
- `src/daemon.rs` — spawn the friday digest scheduler on startup, gated on `config.stonkwatch_social.friday_digest_enabled`
- `src/config.rs` — add `friday_digest_enabled: Option<bool>` to `StonkwatchSocialConfig` (default None = off)
- `src/triggers/mod.rs` — register `pub mod friday_digest;`
- `flake.nix` — ensure `ffmpeg` in runtime (not just devShell)

---

## Task 1: `render_segment` HTTP client

In `chart_video.rs`, add a sibling to `render_announcement_video`. Body identical except the URL path is `/api/v1/chart-video/segment` and the request JSON uses `window_days` + `duration_secs` instead of `window_hours`. `announcement_id` is `Option<&str>` — skip the field when `None` so stonkwatch falls back to the sentiment-based headline.

Commit: `feat(chart-video): add render_segment for weekly digest segments`.

## Task 2: DB helpers for weekly exclude

Add to `src/db.rs`:

```rust
pub async fn tickers_posted_this_iso_week(&self) -> Result<std::collections::HashSet<String>> {
    let monday_ts = current_iso_week_monday_utc_ts()?;
    let conn = self.pool.get().await?;
    let mut rows = conn.query(
        "SELECT DISTINCT ticker FROM trending_posts WHERE posted_at >= ?1 AND ticker != 'WEEKLY_DIGEST'",
        turso::params![monday_ts],
    ).await?;
    let mut set = std::collections::HashSet::new();
    while let Some(row) = rows.next().await? {
        if let Ok(v) = row.get_value(0) {
            if let Some(t) = v.as_text() {
                set.insert(t.to_string());
            }
        }
    }
    Ok(set)
}

pub async fn weekly_digest_posted_this_iso_week(&self) -> Result<bool> {
    let monday_ts = current_iso_week_monday_utc_ts()?;
    let conn = self.pool.get().await?;
    let mut rows = conn.query(
        "SELECT 1 FROM trending_posts WHERE post_type = 'weekly_digest' AND posted_at >= ?1 LIMIT 1",
        turso::params![monday_ts],
    ).await?;
    Ok(rows.next().await?.is_some())
}

fn current_iso_week_monday_utc_ts() -> Result<i64> {
    let now = chrono::Utc::now();
    let iso = now.iso_week();
    let monday = chrono::NaiveDate::from_isoywd_opt(iso.year(), iso.week(), chrono::Weekday::Mon)
        .ok_or_else(|| anyhow::anyhow!("invalid iso week"))?;
    Ok(monday.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp())
}
```

Match the existing DB method style (pool access, error propagation). Commit: `feat(db): iso-week queries for Friday exclude-repeats`.

## Task 3: Stitcher

Create `src/connectors/stonkwatch_social/stitcher.rs`:

```rust
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use tokio::process::Command;

pub async fn concat_mp4s(inputs: &[PathBuf], output_path: &Path) -> Result<()> {
    if inputs.is_empty() {
        anyhow::bail!("no inputs to stitch");
    }
    let tmp_dir = tempfile::tempdir().context("tempdir for ffmpeg list")?;
    let list_path = tmp_dir.path().join("inputs.txt");
    let list_content = inputs.iter()
        .map(|p| format!("file '{}'", p.to_string_lossy().replace('\'', "'\\''")))
        .collect::<Vec<_>>().join("\n");
    tokio::fs::write(&list_path, list_content).await.context("write ffmpeg list")?;

    let status = Command::new("ffmpeg")
        .args(["-y", "-f", "concat", "-safe", "0", "-i",
               &list_path.to_string_lossy(), "-c", "copy"])
        .arg(output_path)
        .output()
        .await
        .context("spawn ffmpeg")?;

    if !status.status.success() {
        let stderr = String::from_utf8_lossy(&status.stderr);
        anyhow::bail!("ffmpeg concat failed: {}", stderr);
    }
    Ok(())
}
```

Register in `mod.rs`: `pub mod stitcher;`.

Commit: `feat(stitcher): ffmpeg concat demuxer for Friday montage`.

## Task 4: Friday digest trigger

Create `src/triggers/friday_digest.rs` with a `run_if_scheduled(&state)` fn that:

1. Returns early unless AEDT weekday = Fri AND hour = 17
2. Returns early if `weekly_digest_posted_this_iso_week()` is true (de-dupe — the scheduler ticks every 5 min so this would otherwise fire multiple times in the hour)
3. Fetches social_cfg; returns early if `chart_video_api_base` or `stonkwatch_api_key` absent
4. Calls `GET /api/v1/trending?timeframe=7d&limit=15` on stonkwatch
5. Excludes tickers in `tickers_posted_this_iso_week()`
6. Takes top 5 remaining; for each fetches the latest announcement via `GET /api/v1/announcements?symbol=X&per_page=1` (best-effort, `None` if no announcement this week)
7. Renders 5 segments in parallel via `futures::future::join_all`
8. Stitches via `stitcher::concat_mp4s`
9. Constructs a `SocialPost` with `video_mp4_path = Some(montage_path)` and text like `"ASX Top 5 · Week of YYYY-MM-DD — $BHP · $CBA · $WDS · $NAB · $FMG"`
10. Finds the tiktok platform in `StonkwatchSocialConnector::build_platforms(&social_cfg)` and calls `publish(&post)`
11. Records a `TrendingPost { ticker: "WEEKLY_DIGEST", post_type: "weekly_digest", ... }` via `state.db.record_trending_post(...)`

Full code in the earlier draft — see commit history if this doc is re-derived. Implementation is routine given the helpers above.

Commit: `feat(friday-digest): cron-driven top-5 weekly montage trigger`.

## Task 5: Spawn from daemon

In `src/daemon.rs`, after existing trigger spawns, add:

```rust
if state.config.read().await.stonkwatch_social.as_ref()
    .and_then(|s| s.friday_digest_enabled)
    .unwrap_or(false)
{
    let state_clone = state.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if let Err(e) = crate::triggers::friday_digest::run_if_scheduled(&state_clone).await {
                tracing::error!(error = %e, "friday_digest tick failed");
            }
        }
    });
}
```

Register the module in `src/triggers/mod.rs`: `pub mod friday_digest;`.

Commit: `feat(daemon): spawn Friday digest scheduler when enabled`.

## Task 6: Config + flake.nix

`src/config.rs` adds `pub friday_digest_enabled: Option<bool>` to `StonkwatchSocialConfig`.

`flake.nix` — verify `pkgs.ffmpeg` in the runtime closure. If only in `devShell`, add to the binary's build inputs.

Commit: `feat(config): friday_digest_enabled flag + ffmpeg in runtime`.

## Verification

- `cargo +nightly check` clean
- `cargo +nightly clippy` no new warnings vs baseline
- Manual on a dev host with ffmpeg: set clock/env to Friday 17:00 AEDT (or just call `execute_friday_digest` directly from a test harness), confirm 5 segments render, ffmpeg stitches, TikTok receives the montage, `trending_posts` row written with `post_type = 'weekly_digest'`.

## Follow-ups

- Add `latest_announcement_id` to stonkwatch's `/api/v1/trending` response so the per-ticker announcement lookup becomes one call instead of N+1 (separate stonkwatch PR).
- Transition cards between segments via ffmpeg complex filter (nice-to-have polish).
- Matrix rollup for render/upload failures (shared with the X/Bluesky plan).
