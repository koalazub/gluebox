use anyhow::{Context, Result};
use tracing::info;

use crate::config::StonkwatchSocialConfig;

const APP_URL: &str = "https://stonkwatch.app";

pub struct PostCandidate {
    pub text: String,
    pub priority: f64,
    pub image_url: Option<String>,
}

pub async fn fetch_post_candidates(config: &StonkwatchSocialConfig) -> Result<Vec<PostCandidate>> {
    let db = libsql::Builder::new_remote(
        config.turso_url.clone(),
        config.turso_auth_token.clone(),
    )
    .build()
    .await
    .context("Failed to connect to Stonkwatch Turso DB")?;

    let conn = db.connect().context("Failed to get connection")?;

    let cutoff = (chrono::Utc::now() - chrono::Duration::hours(6)).timestamp();

    let mut rows = conn
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
        .await
        .context("Failed to query announcements")?;

    let mut candidates = Vec::new();

    while let Some(row) = rows.next().await? {
        let ann_id = row.get::<String>(0).unwrap_or_default();
        let symbol = row.get::<String>(1).unwrap_or_default();
        let title = row.get::<String>(2).unwrap_or_default();
        let ann_type = row.get::<String>(3).unwrap_or_default();
        let is_price_sensitive = row.get::<i64>(4).unwrap_or(0) == 1;
        let summary: Option<String> = row.get::<String>(5).ok();
        let sentiment: Option<String> = row.get::<String>(6).ok();

        let priority = if is_price_sensitive { 2.0 } else { 1.0 };

        let text = format_post(
            &symbol,
            &title,
            &ann_type,
            is_price_sensitive,
            summary.as_deref(),
            sentiment.as_deref(),
            &ann_id,
        );

        candidates.push(PostCandidate {
            text,
            priority,
            image_url: None,
        });
    }

    candidates.sort_by(|a, b| b.priority.partial_cmp(&a.priority).unwrap_or(std::cmp::Ordering::Equal));

    info!(count = candidates.len(), "Fetched post candidates from Stonkwatch");
    Ok(candidates)
}

fn format_post(
    symbol: &str,
    title: &str,
    ann_type: &str,
    is_price_sensitive: bool,
    summary: Option<&str>,
    sentiment: Option<&str>,
    announcement_id: &str,
) -> String {
    let sensitivity = if is_price_sensitive { " ⚡" } else { "" };
    let sentiment_emoji = match sentiment {
        Some(s) if s.contains("positive") => "📈",
        Some(s) if s.contains("negative") => "📉",
        _ => "📊",
    };

    let link = format!("{}/announcement/{}", APP_URL, announcement_id);

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
