use anyhow::Result;
use tracing::info;

use crate::config::StonkwatchSocialConfig;
use crate::connectors::opencode::OpenCodeClient;
use super::pipeline;
use super::platform::SocialPost;

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

    pub fn to_social_post(&self) -> SocialPost {
        SocialPost {
            text: self.text.clone(),
            link: self.link.clone(),
            image_url: self.og_image_path.clone(),
            og_title: self.og_title(),
            og_description: self.og_description(),
        }
    }
}

pub async fn fetch_post_candidates(
    conn: &turso::Connection,
    config: &StonkwatchSocialConfig,
    already_posted: &std::collections::HashSet<String>,
) -> Result<Vec<PostCandidate>> {
    let cutoff = (chrono::Utc::now() - chrono::Duration::hours(24)).timestamp();

    let announcements = pipeline::fetch_announcements(conn, cutoff, already_posted).await?;

    let llm = config.openrouter_api_key.as_ref().map(|key| OpenCodeClient::new(key));
    let mut candidates = Vec::new();

    for ann in &announcements {
        let text = pipeline::generate_post_text(llm.as_ref(), ann).await;

        let og_image_path = pipeline::prepare_image(
            ann,
            std::path::Path::new("/var/lib/gluebox/og-images"),
            config.storj.as_ref(),
        ).await;

        candidates.push(PostCandidate {
            announcement_id: ann.id.clone(),
            symbol: ann.symbol.clone(),
            title: ann.title.clone(),
            ann_type: ann.ann_type.clone(),
            importance: ann.importance.clone(),
            summary: ann.summary.clone(),
            link: ann.link.clone(),
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
