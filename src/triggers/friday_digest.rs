use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::{Datelike, Timelike};
use chrono_tz::Australia::Sydney;
use tracing::{info, warn};

use crate::AppState;
use crate::connectors::stonkwatch_social::{
    StonkwatchSocialConnector,
    chart_video,
    platform::SocialPost,
    stitcher,
};
use crate::db::TrendingPost;

const SEGMENT_DIR: &str = "/var/lib/gluebox/friday-segments";
const MONTAGE_DIR: &str = "/var/lib/gluebox/friday-montage";
const WINDOW_DAYS: u32 = 7;
const SEGMENT_DURATION_SECS: u32 = 8;

#[derive(serde::Deserialize)]
struct TrendingResponse {
    trending: Vec<TrendingItem>,
}

#[derive(serde::Deserialize)]
struct TrendingItem {
    symbol: String,
}

#[derive(serde::Deserialize)]
struct AnnouncementsResponse {
    announcements: Vec<AnnouncementItem>,
}

#[derive(serde::Deserialize)]
struct AnnouncementItem {
    id: String,
}

pub async fn run_if_scheduled(state: &Arc<AppState>) -> Result<()> {
    let enabled = state
        .config
        .read()
        .await
        .stonkwatch_social
        .as_ref()
        .and_then(|s| s.friday_digest_enabled)
        .unwrap_or(false);
    if !enabled {
        return Ok(());
    }

    let now_sydney = chrono::Utc::now().with_timezone(&Sydney);
    if now_sydney.weekday() != chrono::Weekday::Fri
        || now_sydney.hour() != 17
        || now_sydney.minute() >= 5
    {
        return Ok(());
    }

    if state.db.weekly_digest_posted_this_iso_week().await? {
        info!("friday_digest: already posted this week, skipping");
        return Ok(());
    }

    let social_cfg = {
        let cfg = state.config.read().await;
        match cfg.stonkwatch_social.clone() {
            Some(c) => c,
            None => {
                warn!("friday_digest: stonkwatch_social not configured");
                return Ok(());
            }
        }
    };

    let api_base = match &social_cfg.chart_video_api_base {
        Some(b) => b.clone(),
        None => {
            warn!("friday_digest: chart_video_api_base not set");
            return Ok(());
        }
    };
    let api_key = match &social_cfg.stonkwatch_api_key {
        Some(k) => k.clone(),
        None => {
            warn!("friday_digest: stonkwatch_api_key not set");
            return Ok(());
        }
    };

    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .context("failed to build reqwest client")?;

    let trending_symbols = fetch_weekly_trending(&http, &api_base, &api_key).await?;
    let already_posted = state.db.tickers_posted_this_iso_week().await?;

    let candidates: Vec<String> = trending_symbols
        .into_iter()
        .filter(|s| !already_posted.contains(s))
        .take(5)
        .collect();

    if candidates.is_empty() {
        info!("friday_digest: no new candidates after exclude-repeats");
        return Ok(());
    }

    let segment_dir = PathBuf::from(SEGMENT_DIR);
    tokio::fs::create_dir_all(&segment_dir)
        .await
        .context("failed to create segment dir")?;

    let render_futs: Vec<_> = candidates
        .iter()
        .map(|symbol| {
            let http = http.clone();
            let api_base = api_base.clone();
            let api_key = api_key.clone();
            let symbol = symbol.clone();
            let segment_dir = segment_dir.clone();
            async move {
                let announcement_id =
                    fetch_latest_announcement_id(&http, &api_base, &api_key, &symbol).await;
                chart_video::render_segment(
                    &api_base,
                    &api_key,
                    &symbol,
                    announcement_id.as_deref(),
                    WINDOW_DAYS,
                    SEGMENT_DURATION_SECS,
                    &segment_dir,
                )
                .await
            }
        })
        .collect();

    let render_results = futures::future::join_all(render_futs).await;

    let mut rendered: Vec<(String, PathBuf)> = Vec::new();
    for (i, result) in render_results.into_iter().enumerate() {
        match result {
            Ok(path) => rendered.push((candidates[i].clone(), path)),
            Err(e) => warn!(symbol = candidates[i], error = %e, "friday_digest: segment render failed"),
        }
    }

    if rendered.is_empty() {
        warn!("friday_digest: all segment renders failed");
        return Ok(());
    }

    let montage_dir = PathBuf::from(MONTAGE_DIR);
    tokio::fs::create_dir_all(&montage_dir)
        .await
        .context("failed to create montage dir")?;
    let montage_path = montage_dir.join(format!(
        "montage-{}.mp4",
        chrono::Utc::now().timestamp(),
    ));

    let segment_paths: Vec<PathBuf> = rendered.iter().map(|(_, p)| p.clone()).collect();
    stitcher::concat_mp4s(&segment_paths, &montage_path).await?;

    for (_, seg) in &rendered {
        if let Err(e) = tokio::fs::remove_file(seg).await {
            warn!(path = %seg.display(), error = %e, "friday_digest: failed to remove segment after stitch");
        }
    }

    let week_date = chrono::Utc::now().date_naive().to_string();
    let ticker_tags = rendered
        .iter()
        .map(|(s, _)| format!("${s}"))
        .collect::<Vec<_>>()
        .join(" · ");
    let post_text = format!("ASX Top 5 · Week of {week_date} — {ticker_tags}");

    let social_post = SocialPost {
        text: post_text,
        link: "https://stonkwatch.app/feed".to_string(),
        image_url: None,
        story_image_url: None,
        og_title: "ASX Weekly Top 5".to_string(),
        og_description: "The week's top-trending ASX stocks, visualised.".to_string(),
        video_mp4_path: Some(montage_path.clone()),
    };

    let platforms = StonkwatchSocialConnector::build_platforms(&social_cfg);
    let mut published = false;
    for platform in &platforms {
        if !platform.accepts(&social_post) {
            continue;
        }
        match platform.publish(&social_post).await {
            Ok(result) => {
                info!(platform = result.platform, id = %result.id, "friday_digest: montage posted");
                published = true;
            }
            Err(e) => warn!(platform = platform.name(), error = %e, "friday_digest: platform publish failed"),
        }
    }

    if !published {
        warn!("friday_digest: montage rendered but no platform accepted/succeeded");
        return Ok(());
    }

    let record = TrendingPost {
        ticker: "WEEKLY_DIGEST".to_string(),
        posted_at: chrono::Utc::now().timestamp(),
        post_type: "weekly_digest".to_string(),
        announcement_id: None,
        stonkwatch_link: "https://stonkwatch.app/feed".to_string(),
    };
    if let Err(e) = state.db.record_trending_post(&record).await {
        warn!(error = %e, "friday_digest: failed to record post log");
    }

    info!("friday_digest: weekly montage complete");
    Ok(())
}

async fn fetch_weekly_trending(
    client: &reqwest::Client,
    api_base: &str,
    api_key: &str,
) -> Result<Vec<String>> {
    let url = format!(
        "{}/api/v1/trending?timeframe=7d&limit=15",
        api_base.trim_end_matches('/')
    );
    let resp = client
        .get(&url)
        .header("X-API-Key", api_key)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("trending fetch returned {}: {}", status, text);
    }

    let parsed: TrendingResponse = resp.json().await?;
    Ok(parsed.trending.into_iter().map(|t| t.symbol).collect())
}

async fn fetch_latest_announcement_id(
    client: &reqwest::Client,
    api_base: &str,
    api_key: &str,
    symbol: &str,
) -> Option<String> {
    let url = format!(
        "{}/api/v1/announcements?symbol={}&per_page=1",
        api_base.trim_end_matches('/'),
        urlencoding::encode(symbol),
    );
    let resp = client
        .get(&url)
        .header("X-API-Key", api_key)
        .send()
        .await
        .ok()?;

    if !resp.status().is_success() {
        return None;
    }

    let parsed: AnnouncementsResponse = resp.json().await.ok()?;
    parsed.announcements.into_iter().next().map(|a| a.id)
}
