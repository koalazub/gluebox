use std::sync::Arc;
use tracing::{info, warn};

use crate::AppState;
use crate::config::StonkwatchSocialConfig;
use crate::connectors::stonkwatch_social::{
    StonkwatchSocialConnector,
    content::PostCandidate,
    pipeline,
    platform::SocialPlatform,
};
use crate::db::TrendingPost;

const DAILY_POST_CAP: i64 = 4;
const PER_TICKER_COOLDOWN_SECS: i64 = 86400;
const ANNOUNCEMENT_MAX_AGE_SECS: i64 = 86400;
const OG_IMAGE_DIR: &str = "/var/lib/gluebox/og-images";

pub async fn handle_trending_entity(
    state: &Arc<AppState>,
    entity_type: &str,
    entity_id: &str,
) -> anyhow::Result<TrendingDecision> {
    if entity_type != "stock" {
        return Ok(TrendingDecision::Skipped("non-stock entity".into()));
    }

    let social_cfg = {
        let cfg = state.config.read().await;
        match cfg.stonkwatch_social.clone() {
            Some(c) => c,
            None => return Ok(TrendingDecision::Skipped("stonkwatch_social not configured".into())),
        }
    };

    let daily_count = state.db.trending_posts_in_last_24h().await.unwrap_or(0);
    if daily_count >= DAILY_POST_CAP {
        info!(ticker = entity_id, daily_count, "trending: daily cap reached, skipping");
        return Ok(TrendingDecision::Skipped("daily cap reached".into()));
    }

    if let Ok(Some(last)) = state.db.last_trending_post_for_ticker(entity_id).await {
        let now = chrono::Utc::now().timestamp();
        if now - last < PER_TICKER_COOLDOWN_SECS {
            info!(ticker = entity_id, age_secs = now - last, "trending: ticker cooldown active, skipping");
            return Ok(TrendingDecision::Skipped("ticker cooldown".into()));
        }
    }

    let announcement = match fetch_announcement(&social_cfg, entity_id).await {
        Ok(Some(a)) => a,
        Ok(None) => {
            info!(ticker = entity_id, "trending: no recent announcement, skipping");
            return Ok(TrendingDecision::Skipped("no recent announcement".into()));
        }
        Err(e) => {
            warn!(ticker = entity_id, error = %e, "trending: failed to fetch announcement");
            return Err(e);
        }
    };

    let candidate = build_candidate(&social_cfg, announcement).await;
    let social_post = candidate.to_social_post();
    let platforms = StonkwatchSocialConnector::build_platforms(&social_cfg);

    if platforms.is_empty() {
        return Ok(TrendingDecision::Skipped("no platforms configured".into()));
    }

    let mut published_somewhere = false;
    for platform in &platforms {
        match platform.publish(&social_post).await {
            Ok(result) => {
                info!(ticker = entity_id, platform = result.platform, id = %result.id, "trending: posted");
                published_somewhere = true;
            }
            Err(e) => warn!(ticker = entity_id, platform = platform.name(), error = %e, "trending: platform publish failed"),
        }
    }

    if !published_somewhere {
        return Ok(TrendingDecision::Skipped("all platforms failed".into()));
    }

    let record = TrendingPost {
        ticker: entity_id.to_string(),
        posted_at: chrono::Utc::now().timestamp(),
        post_type: "announcement".into(),
        announcement_id: Some(candidate.announcement_id.clone()),
        stonkwatch_link: candidate.link.clone(),
    };
    if let Err(e) = state.db.record_trending_post(&record).await {
        warn!(ticker = entity_id, error = %e, "trending: failed to record post log");
    }

    Ok(TrendingDecision::Posted { ticker: entity_id.to_string(), announcement_id: candidate.announcement_id })
}

async fn fetch_announcement(
    social_cfg: &StonkwatchSocialConfig,
    ticker: &str,
) -> anyhow::Result<Option<pipeline::AnnouncementData>> {
    let db = turso::sync::Builder::new_remote("/var/lib/gluebox/stonkwatch-replica")
        .with_remote_url(&social_cfg.turso_url)
        .with_auth_token(&social_cfg.turso_auth_token)
        .build()
        .await
        .map_err(|e| anyhow::anyhow!("open stonkwatch turso: {e}"))?;

    let conn = db.connect().await.map_err(|e| anyhow::anyhow!("turso connect: {e}"))?;
    pipeline::fetch_latest_for_ticker(&conn, ticker, ANNOUNCEMENT_MAX_AGE_SECS).await
}

async fn build_candidate(
    social_cfg: &StonkwatchSocialConfig,
    ann: pipeline::AnnouncementData,
) -> PostCandidate {
    let llm = social_cfg.openrouter_api_key.as_ref()
        .map(|key| crate::connectors::opencode::OpenCodeClient::new(key));
    let text = pipeline::generate_post_text(llm.as_ref(), &ann).await;

    let image_ann = if ann.summary.is_none() {
        let mut enriched = ann.clone();
        let clean_text = text.split("https://").next().unwrap_or(&text).trim();
        enriched.summary = Some(clean_text.to_string());
        enriched
    } else {
        ann.clone()
    };

    let output_dir = std::path::Path::new(OG_IMAGE_DIR);
    let og_image_path = pipeline::prepare_image(&image_ann, output_dir, social_cfg.storj.as_ref()).await;
    let story_image_path = pipeline::prepare_story_image(&image_ann, output_dir, social_cfg.storj.as_ref()).await;
    let video_mp4_path = pipeline::prepare_chart_video(
        &ann,
        output_dir,
        social_cfg.chart_video_api_base.as_deref(),
        social_cfg.stonkwatch_api_key.as_deref(),
    )
    .await;

    PostCandidate {
        announcement_id: ann.id.clone(),
        symbol: ann.symbol.clone(),
        title: ann.title.clone(),
        ann_type: ann.ann_type.clone(),
        importance: ann.importance.clone(),
        summary: ann.summary.clone(),
        link: ann.link.clone(),
        text,
        og_image_path,
        story_image_path,
        video_mp4_path,
    }
}

#[derive(Debug)]
pub enum TrendingDecision {
    Posted { ticker: String, announcement_id: String },
    Skipped(String),
}

impl TrendingDecision {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Posted { .. } => "posted",
            Self::Skipped(_) => "skipped",
        }
    }
}
