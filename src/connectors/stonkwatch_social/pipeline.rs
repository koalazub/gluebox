use anyhow::{Context, Result};
use std::sync::Arc;
use tracing::warn;

use crate::connectors::opencode::OpenCodeClient;
use crate::triggers::error_rollup::ErrorRollup;

const APP_URL: &str = "https://stonkwatch.app";

const SYSTEM_PROMPT: &str = r#"You are the social media voice for Stonkwatch — an ASX market intelligence platform built in Australia.

VOICE: Sharp, informed, no-bullshit finance commentary. Think a seasoned market analyst who's also good at Twitter. You understand what matters to retail investors and cut straight to it.

HARD RULES:
- Max 280 characters total including the link
- $SYMBOL format for tickers, always
- Link goes at the end, always
- NEVER give financial advice, recommendations, or say buy/sell/hold
- NEVER use hashtags
- NEVER use emojis except 📈📉⚡ sparingly
- Australian English spelling (analyse, behaviour, colour)
- Output ONLY the post text. No quotes, no meta-commentary, no "here's a post"

WHAT MAKES A GOOD POST:
- Lead with the insight, not the event. "BHP's iron ore output beat estimates by 12%" not "BHP released their quarterly report"
- If price-sensitive, convey urgency: "⚡ $BHP just dropped..."
- If there's an AI summary, extract the single most interesting finding
- Vary structure: sometimes a single punchy sentence, sometimes two short ones, sometimes a question

BAD POSTS (never write these):
- "Breaking: $BHP has released their quarterly results. Click here to learn more"
- "$BHP quarterly update now available on Stonkwatch! 📊🚀 #ASX #BHP"
- "Exciting news from $BHP! Check out the full AI summary on Stonkwatch"

GOOD POSTS (aim for this quality):
- "⚡ $BHP iron ore output up 12% QoQ — biggest beat in 3 quarters. Full AI breakdown:"
- "$LYC rare earths guidance slashed. Sentiment across Reddit and HotCopper turned bearish in hours."
- "$CBA dropped a price-sensitive on mortgage arrears. The numbers aren't great."
- "Three ASX lithium plays filed updates today. Only one had good news."#;

#[derive(Clone)]
pub struct AnnouncementData {
    pub id: String,
    pub symbol: String,
    pub title: String,
    pub ann_type: String,
    pub importance: String,
    pub summary: Option<String>,
    pub link: String,
}

impl AnnouncementData {
    pub fn is_price_sensitive(&self) -> bool {
        self.ann_type == "price_sensitive" || self.importance == "high"
    }

}

pub async fn fetch_announcements(
    conn: &turso::Connection,
    cutoff: i64,
    already_posted: &std::collections::HashSet<String>,
) -> Result<Vec<AnnouncementData>> {
    let mut rows = conn
        .query(
            "SELECT ca.id, ca.symbol, ca.title, ca.announcement_type, ca.importance,
                    ai.summary_text
             FROM company_announcements ca
             LEFT JOIN ai_summaries ai ON ca.id = ai.announcement_id
             WHERE ca.published_at >= ?1
             ORDER BY ca.importance DESC, ca.published_at DESC
             LIMIT 20",
            (cutoff,),
        )
        .await
        .map_err(|e| anyhow::anyhow!("Failed to query announcements: {}", e))?;

    let mut announcements = Vec::new();

    while let Some(row) = rows.next().await.context("Failed to iterate rows")? {
        let id = row.get_value(0).ok().and_then(|v| v.as_text().map(|s| s.to_string())).unwrap_or_default();
        if already_posted.contains(&id) {
            continue;
        }

        let symbol = row.get_value(1).ok().and_then(|v| v.as_text().map(|s| s.to_string())).unwrap_or_default();
        let title = row.get_value(2).ok().and_then(|v| v.as_text().map(|s| s.to_string())).unwrap_or_default();
        let ann_type = row.get_value(3).ok().and_then(|v| v.as_text().map(|s| s.to_string())).unwrap_or_default();
        let importance = row.get_value(4).ok().and_then(|v| v.as_text().map(|s| s.to_string())).unwrap_or_default();
        let summary = row.get_value(5).ok().and_then(|v| v.as_text().map(|s| s.to_string()));

        let link = format!("{}/announcement/{}", APP_URL, id);

        announcements.push(AnnouncementData {
            id,
            symbol,
            title,
            ann_type,
            importance,
            summary,
            link,
        });
    }

    Ok(announcements)
}

pub async fn fetch_latest_for_ticker(
    conn: &turso::Connection,
    ticker: &str,
    max_age_secs: i64,
) -> Result<Option<AnnouncementData>> {
    let cutoff = chrono::Utc::now().timestamp() - max_age_secs;
    let mut rows = conn
        .query(
            "SELECT ca.id, ca.symbol, ca.title, ca.announcement_type, ca.importance,
                    ai.summary_text
             FROM company_announcements ca
             LEFT JOIN ai_summaries ai ON ca.id = ai.announcement_id
             WHERE ca.symbol = ?1 AND ca.published_at >= ?2
             ORDER BY ca.importance DESC, ca.published_at DESC
             LIMIT 1",
            (ticker, cutoff),
        )
        .await
        .map_err(|e| anyhow::anyhow!("Failed to query announcement for ticker {ticker}: {e}"))?;

    let Some(row) = rows.next().await.context("row iter failed")? else {
        return Ok(None);
    };

    let id = row.get_value(0).ok().and_then(|v| v.as_text().map(|s| s.to_string())).unwrap_or_default();
    let symbol = row.get_value(1).ok().and_then(|v| v.as_text().map(|s| s.to_string())).unwrap_or_default();
    let title = row.get_value(2).ok().and_then(|v| v.as_text().map(|s| s.to_string())).unwrap_or_default();
    let ann_type = row.get_value(3).ok().and_then(|v| v.as_text().map(|s| s.to_string())).unwrap_or_default();
    let importance = row.get_value(4).ok().and_then(|v| v.as_text().map(|s| s.to_string())).unwrap_or_default();
    let summary = row.get_value(5).ok().and_then(|v| v.as_text().map(|s| s.to_string()));
    let link = format!("{}/announcement/{}", APP_URL, id);

    Ok(Some(AnnouncementData {
        id,
        symbol,
        title,
        ann_type,
        importance,
        summary,
        link,
    }))
}

pub async fn generate_post_text(
    llm: Option<&OpenCodeClient>,
    announcement: &AnnouncementData,
) -> String {
    if let Some(client) = llm {
        match generate_post_llm(client, announcement).await {
            Ok(post) => post,
            Err(e) => {
                warn!(symbol = announcement.symbol, error = %e, "LLM post generation failed, using fallback");
                fallback_post(announcement)
            }
        }
    } else {
        fallback_post(announcement)
    }
}

async fn generate_post_llm(
    client: &OpenCodeClient,
    ann: &AnnouncementData,
) -> Result<String> {
    let is_price_sensitive = ann.is_price_sensitive();
    let mut context = format!(
        "Stock: ${}\nAnnouncement type: {}\nTitle: {}\nImportance: {}\nPrice sensitive: {}\nLink: {}",
        ann.symbol, ann.ann_type, ann.title, ann.importance,
        if is_price_sensitive { "YES" } else { "no" },
        ann.link,
    );

    if let Some(ref s) = ann.summary {
        context.push_str(&format!("\nAI Summary: {}", s));
    }

    let user_prompt = format!(
        "{}\n\nWrite a single social media post (max 280 chars) for this announcement. Include the link at the end. Output ONLY the post text, nothing else.",
        context
    );

    let response = client.chat(SYSTEM_PROMPT, &user_prompt, 200).await?;
    let post = response.trim().trim_matches('"').to_string();

    if post.len() > 300 {
        anyhow::bail!("Generated post too long: {} chars", post.len());
    }

    if !post.contains(&ann.link) {
        return Ok(format!("{}\n\n{}", post.trim(), ann.link));
    }

    Ok(post)
}

fn fallback_post(ann: &AnnouncementData) -> String {
    let is_price_sensitive = ann.is_price_sensitive();
    let sensitivity = if is_price_sensitive { " ⚡" } else { "" };

    let title_truncated = if ann.title.len() > 100 {
        format!("{}...", &ann.title[..97])
    } else {
        ann.title.clone()
    };

    let header = format!("${}{} — {}", ann.symbol, sensitivity, ann.ann_type);

    if let Some(ref summary_text) = ann.summary {
        let summary_short = if summary_text.len() > 120 {
            format!("{}...", &summary_text[..117])
        } else {
            summary_text.clone()
        };

        let with_summary = format!("{}\n\n{}\n\n{}\n\n{}", header, title_truncated, summary_short, ann.link);
        if with_summary.len() <= 300 {
            return with_summary;
        }
    }

    format!("{}\n\n{}\n\n{}", header, title_truncated, ann.link)
}

pub async fn prepare_image(
    announcement: &AnnouncementData,
    output_dir: &std::path::Path,
    storj_config: Option<&crate::config::StorjConfig>,
) -> Option<String> {
    match super::og_image::generate_og_image(
        &announcement.symbol,
        &announcement.title,
        &announcement.ann_type,
        announcement.is_price_sensitive(),
        announcement.summary.as_deref().unwrap_or_default(),
        &announcement.id,
        output_dir,
    ).await {
        Ok(local_path) => {
            if let Some(storj_cfg) = storj_config {
                let object_key = format!("og/{}.png", announcement.id);
                match super::storj::upload_image(storj_cfg, &local_path.display().to_string(), &object_key).await {
                    Ok(public_url) => Some(public_url),
                    Err(e) => {
                        warn!(symbol = announcement.symbol, error = %e, "Storj upload failed, using local path");
                        Some(local_path.display().to_string())
                    }
                }
            } else {
                Some(local_path.display().to_string())
            }
        }
        Err(e) => {
            warn!(symbol = announcement.symbol, error = %e, "OG image generation failed");
            None
        }
    }
}

pub async fn prepare_story_image(
    announcement: &AnnouncementData,
    output_dir: &std::path::Path,
    storj_config: Option<&crate::config::StorjConfig>,
) -> Option<String> {
    match super::og_image::generate_story_image(
        &announcement.symbol,
        &announcement.title,
        &announcement.ann_type,
        announcement.is_price_sensitive(),
        announcement.summary.as_deref().unwrap_or_default(),
        &announcement.id,
        &announcement.link,
        output_dir,
    ).await {
        Ok(local_path) => {
            if let Some(storj_cfg) = storj_config {
                let object_key = format!("og/{}-story.png", announcement.id);
                match super::storj::upload_image(storj_cfg, &local_path.display().to_string(), &object_key).await {
                    Ok(public_url) => Some(public_url),
                    Err(e) => {
                        warn!(symbol = announcement.symbol, error = %e, "Storj story upload failed");
                        None
                    }
                }
            } else {
                Some(local_path.display().to_string())
            }
        }
        Err(e) => {
            warn!(symbol = announcement.symbol, error = %e, "Story image generation failed");
            None
        }
    }
}

pub async fn prepare_chart_video(
    ann: &AnnouncementData,
    output_dir: &std::path::Path,
    api_base: Option<&str>,
    api_key: Option<&str>,
    error_rollup: &Arc<ErrorRollup>,
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
            error_rollup.record("chart_video_render_failed", format!("{}: {}", ann.symbol, e)).await;
            None
        }
    }
}
