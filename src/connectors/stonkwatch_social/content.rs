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
    pub announcement_id: String,
    pub symbol: String,
    pub title: String,
    pub ann_type: String,
    pub importance: String,
    pub summary: Option<String>,
    pub link: String,
    pub text: String,
    pub og_image_path: Option<String>,
}

impl PostCandidate {
    pub fn is_price_sensitive(&self) -> bool {
        self.ann_type == "price_sensitive" || self.importance == "high"
    }

    pub fn og_title(&self) -> String {
        let marker = if self.is_price_sensitive() { " ⚡" } else { "" };
        format!("${}{} — {}", self.symbol, marker, self.title)
    }

    pub fn og_description(&self) -> String {
        match &self.summary {
            Some(s) if s.len() > 200 => format!("{}...", &s[..197]),
            Some(s) => s.clone(),
            None => format!("{} announcement from {} on ASX", self.ann_type, self.symbol),
        }
    }
}

pub async fn fetch_post_candidates(
    conn: &turso::Connection,
    config: &StonkwatchSocialConfig,
    already_posted: &std::collections::HashSet<String>,
) -> Result<Vec<PostCandidate>> {
    let cutoff = (chrono::Utc::now() - chrono::Duration::hours(24)).timestamp();

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
        .map_err(|e| {
            tracing::error!(error = %e, url = %config.turso_url, "Turso query failed");
            anyhow::anyhow!("Failed to query announcements: {}", e)
        })?;

    let llm = config.openrouter_api_key.as_ref().map(|key| OpenCodeClient::new(key));
    let mut candidates = Vec::new();

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

        let link = format!("{}/announcement/{}?utm_source=social&utm_medium=bot", APP_URL, id);

        let is_price_sensitive = ann_type == "price_sensitive" || importance == "high";

        let text = if let Some(ref client) = llm {
            match generate_post(client, &symbol, &title, &ann_type, &importance, summary.as_deref(), &link).await {
                Ok(post) => post,
                Err(e) => {
                    warn!(symbol, error = %e, "LLM post generation failed, using fallback");
                    fallback_post(&symbol, &title, &ann_type, is_price_sensitive, summary.as_deref(), &link)
                }
            }
        } else {
            fallback_post(&symbol, &title, &ann_type, is_price_sensitive, summary.as_deref(), &link)
        };

        let og_data = super::og_image::OgCardData {
            symbol: symbol.clone(),
            title: title.clone(),
            ann_type: ann_type.clone(),
            is_price_sensitive,
            sentiment: String::new(),
            summary: summary.clone().unwrap_or_default(),
            announcement_id: id.clone(),
        };

        let og_image_path = match super::og_image::generate_og_image(
            &og_data,
            std::path::Path::new("/var/lib/gluebox/og-images"),
        ).await {
            Ok(local_path) => {
                if let Some(ref storj_cfg) = config.storj {
                    let object_key = format!("og/{}.png", id);
                    match super::storj::upload_image(storj_cfg, &local_path.display().to_string(), &object_key).await {
                        Ok(public_url) => Some(public_url),
                        Err(e) => {
                            warn!(symbol, error = %e, "Storj upload failed, using local path");
                            Some(local_path.display().to_string())
                        }
                    }
                } else {
                    Some(local_path.display().to_string())
                }
            }
            Err(e) => {
                warn!(symbol, error = %e, "OG image generation failed");
                None
            }
        };

        candidates.push(PostCandidate {
            announcement_id: id,
            symbol,
            title,
            ann_type,
            importance,
            summary,
            link,
            text,
            og_image_path,
        });
    }

    candidates.sort_by(|a, b| {
        let pa: f64 = if a.is_price_sensitive() { 2.0 } else { 1.0 };
        let pb: f64 = if b.is_price_sensitive() { 2.0 } else { 1.0 };
        pb.partial_cmp(&pa).unwrap_or(std::cmp::Ordering::Equal)
    });

    if !candidates.is_empty() {
        info!(count = candidates.len(), "Prepared post candidates");
    } else {
        info!("No new announcements in the last 24 hours");
    }

    Ok(candidates)
}

async fn generate_post(
    client: &OpenCodeClient,
    symbol: &str,
    title: &str,
    ann_type: &str,
    importance: &str,
    summary: Option<&str>,
    link: &str,
) -> Result<String> {
    let is_price_sensitive = ann_type == "price_sensitive" || importance == "high";
    let mut context = format!(
        "Stock: ${}\nAnnouncement type: {}\nTitle: {}\nImportance: {}\nPrice sensitive: {}\nLink: {}",
        symbol, ann_type, title, importance,
        if is_price_sensitive { "YES" } else { "no" },
        link,
    );

    if let Some(s) = summary {
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

    if !post.contains(link) {
        return Ok(format!("{}\n\n{}", post.trim(), link));
    }

    Ok(post)
}

fn fallback_post(
    symbol: &str,
    title: &str,
    ann_type: &str,
    is_price_sensitive: bool,
    summary: Option<&str>,
    link: &str,
) -> String {
    let sensitivity = if is_price_sensitive { " ⚡" } else { "" };

    let title_truncated = if title.len() > 100 {
        format!("{}...", &title[..97])
    } else {
        title.to_string()
    };

    let header = format!("${}{} — {}", symbol, sensitivity, ann_type);

    if let Some(summary_text) = summary {
        let summary_short = if summary_text.len() > 120 {
            format!("{}...", &summary_text[..117])
        } else {
            summary_text.to_string()
        };

        let with_summary = format!("{}\n\n{}\n\n{}\n\n{}", header, title_truncated, summary_short, link);
        if with_summary.len() <= 300 {
            return with_summary;
        }
    }

    format!("{}\n\n{}\n\n{}", header, title_truncated, link)
}
