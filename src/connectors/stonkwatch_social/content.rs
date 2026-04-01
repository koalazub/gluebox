use anyhow::{Context, Result};
use tracing::{info, warn};

use crate::config::StonkwatchSocialConfig;
use crate::connectors::opencode::OpenCodeClient;

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
- If there's sentiment data, weave it in naturally: "...and the market's not buying it"
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

pub struct PostCandidate {
    pub text: String,
    pub priority: f64,
    pub image_url: Option<String>,
}

struct AnnouncementData {
    id: String,
    symbol: String,
    title: String,
    ann_type: String,
    is_price_sensitive: bool,
    summary: Option<String>,
    sentiment: Option<String>,
}

pub async fn fetch_post_candidates(config: &StonkwatchSocialConfig) -> Result<Vec<PostCandidate>> {
    let url = config.turso_url.replace("libsql://", "https://");
    info!(url = %url, "Connecting to Stonkwatch Turso DB");
    let db = libsql::Builder::new_remote(url.clone(), config.turso_auth_token.clone())
        .build()
        .await
        .with_context(|| format!("Failed to connect to Stonkwatch Turso DB at {}", url))?;

    let conn = db.connect().context("Failed to get Turso connection")?;

    let cutoff = (chrono::Utc::now() - chrono::Duration::hours(6)).timestamp();

    let mut rows = match conn
        .query(
            "SELECT ca.id, ca.symbol, ca.title, ca.announcement_type, ca.is_price_sensitive,
                    ai.summary_text, ai.sentiment, ai.financial_impact
             FROM company_announcements ca
             LEFT JOIN ai_summaries ai ON ca.id = ai.announcement_id
             WHERE ca.published_at >= ?1
             ORDER BY ca.is_price_sensitive DESC, ca.published_at DESC
             LIMIT 20",
            libsql::params![cutoff],
        )
        .await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, url = %config.turso_url, "Turso query failed");
            anyhow::bail!("Failed to query announcements: {}", e);
        }
    };

    let mut announcements = Vec::new();

    while let Some(row) = rows.next().await? {
        announcements.push(AnnouncementData {
            id: row.get::<String>(0).unwrap_or_default(),
            symbol: row.get::<String>(1).unwrap_or_default(),
            title: row.get::<String>(2).unwrap_or_default(),
            ann_type: row.get::<String>(3).unwrap_or_default(),
            is_price_sensitive: row.get::<i64>(4).unwrap_or(0) == 1,
            summary: row.get::<String>(5).ok(),
            sentiment: row.get::<String>(6).ok(),
        });
    }

    if announcements.is_empty() {
        info!("No announcements found in the last 6 hours");
        return Ok(Vec::new());
    }

    let llm = config.openrouter_api_key.as_ref().map(|key| OpenCodeClient::new(key));

    let mut candidates = Vec::new();

    for ann in &announcements {
        let priority = if ann.is_price_sensitive { 2.0 } else { 1.0 };
        let link = format!("{}/announcement/{}?utm_source=social&utm_medium=bot", APP_URL, ann.id);

        let text = if let Some(ref client) = llm {
            match generate_post(client, ann, &link).await {
                Ok(post) => post,
                Err(e) => {
                    warn!(symbol = ann.symbol, error = %e, "LLM post generation failed, using fallback");
                    fallback_post(ann, &link)
                }
            }
        } else {
            fallback_post(ann, &link)
        };

        let og_data = super::og_image::OgCardData {
            symbol: ann.symbol.clone(),
            title: ann.title.clone(),
            ann_type: ann.ann_type.clone(),
            is_price_sensitive: ann.is_price_sensitive,
            sentiment: ann.sentiment.clone().unwrap_or_default(),
            summary: ann.summary.clone().unwrap_or_default(),
            announcement_id: ann.id.clone(),
        };

        let image_path = match super::og_image::generate_og_image(
            &og_data,
            std::path::Path::new("/var/lib/gluebox/og-images"),
        )
        .await
        {
            Ok(path) => Some(path.display().to_string()),
            Err(e) => {
                warn!(symbol = ann.symbol, error = %e, "OG image generation failed");
                None
            }
        };

        candidates.push(PostCandidate {
            text,
            priority,
            image_url: image_path,
        });
    }

    candidates.sort_by(|a, b| b.priority.partial_cmp(&a.priority).unwrap_or(std::cmp::Ordering::Equal));

    info!(count = candidates.len(), "Prepared post candidates");
    Ok(candidates)
}

async fn generate_post(client: &OpenCodeClient, ann: &AnnouncementData, link: &str) -> Result<String> {
    let mut context = format!(
        "Stock: ${}\nAnnouncement type: {}\nTitle: {}\nPrice sensitive: {}\nLink: {}",
        ann.symbol, ann.ann_type, ann.title,
        if ann.is_price_sensitive { "YES" } else { "no" },
        link,
    );

    if let Some(ref summary) = ann.summary {
        context.push_str(&format!("\nAI Summary: {}", summary));
    }
    if let Some(ref sentiment) = ann.sentiment {
        context.push_str(&format!("\nSentiment: {}", sentiment));
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

    if !post.contains(link) {
        return Ok(format!("{}\n\n{}", post.trim(), link));
    }

    Ok(post)
}

fn fallback_post(ann: &AnnouncementData, link: &str) -> String {
    let sensitivity = if ann.is_price_sensitive { " ⚡" } else { "" };
    let sentiment_emoji = match ann.sentiment.as_deref() {
        Some(s) if s.contains("positive") => "📈",
        Some(s) if s.contains("negative") => "📉",
        _ => "📊",
    };

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

        let with_summary = format!(
            "{}\n\n{}\n\n{} {}\n\n{}",
            header, title_truncated, sentiment_emoji, summary_short, link
        );

        if with_summary.len() <= 300 {
            return with_summary;
        }
    }

    format!("{}\n\n{}\n\n{}", header, title_truncated, link)
}
