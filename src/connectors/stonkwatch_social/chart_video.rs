use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize)]
struct GenerateSegmentVideoRequest<'a> {
    symbol: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    announcement_id: Option<&'a str>,
    window_days: u32,
    duration_secs: u32,
    profile: &'a str,
}

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

fn slug(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

pub async fn render_announcement_video(
    api_base: &str,
    api_key: &str,
    symbol: &str,
    announcement_id: &str,
    output_dir: &Path,
) -> Result<PathBuf> {
    let url = format!(
        "{}/api/v1/chart-video/announcement",
        api_base.trim_end_matches('/')
    );
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

    tokio::fs::create_dir_all(output_dir)
        .await
        .context("failed to create announcement output dir")?;
    let out_path = output_dir.join(format!(
        "{}-{}-{}.mp4",
        slug(symbol),
        slug(announcement_id),
        chrono::Utc::now().timestamp(),
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

pub async fn render_segment(
    api_base: &str,
    api_key: &str,
    symbol: &str,
    announcement_id: Option<&str>,
    window_days: u32,
    duration_secs: u32,
    output_dir: &Path,
) -> Result<PathBuf> {
    let url = format!(
        "{}/api/v1/chart-video/segment",
        api_base.trim_end_matches('/')
    );
    let body = GenerateSegmentVideoRequest {
        symbol,
        announcement_id,
        window_days,
        duration_secs,
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
        .context("chart-video segment render request failed")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("chart-video segment render returned {}: {}", status, text);
    }

    let parsed: GenerateVideoResponse = resp
        .json()
        .await
        .context("chart-video segment render returned malformed JSON")?;

    tokio::fs::create_dir_all(output_dir)
        .await
        .context("failed to create segment output dir")?;
    let out_path = output_dir.join(format!(
        "segment-{}-{}.mp4",
        slug(symbol),
        chrono::Utc::now().timestamp(),
    ));

    let mp4_bytes = client
        .get(&parsed.video_url)
        .send()
        .await
        .context("segment mp4 download failed")?
        .error_for_status()
        .context("segment mp4 download non-2xx")?
        .bytes()
        .await
        .context("segment mp4 body read failed")?;

    tokio::fs::write(&out_path, &mp4_bytes)
        .await
        .context("segment mp4 persist failed")?;

    Ok(out_path)
}
