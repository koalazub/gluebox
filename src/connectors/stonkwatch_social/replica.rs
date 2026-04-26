use anyhow::Context;
use tracing::{debug, info, warn};
use turso::sync::{PartialBootstrapStrategy, PartialSyncOpts};

use crate::config::StonkwatchSocialConfig;

const REPLICA_PATH: &str = "/var/lib/gluebox/stonkwatch-replica";

// Only the last 7 days of announcements + their ai_summaries are needed by the
// social posting trigger. Subsetting the bootstrap to those rows keeps the
// replica small and the first pull fast — full DB has much more we never read.
const PARTIAL_BOOTSTRAP_QUERY: &str =
    "SELECT 1 FROM company_announcements WHERE published_at > strftime('%s','now','-7 days') \
     UNION ALL \
     SELECT 1 FROM ai_summaries WHERE created_at > strftime('%s','now','-7 days')";

const PARTIAL_SEGMENT_SIZE: usize = 128 * 1024;

pub async fn open_synced_replica(
    cfg: &StonkwatchSocialConfig,
) -> anyhow::Result<turso::sync::Database> {
    let partial_opts = PartialSyncOpts {
        bootstrap_strategy: Some(PartialBootstrapStrategy::Query {
            query: PARTIAL_BOOTSTRAP_QUERY.to_string(),
        }),
        segment_size: PARTIAL_SEGMENT_SIZE,
        prefetch: true,
    };

    let db = turso::sync::Builder::new_remote(REPLICA_PATH)
        .with_remote_url(&cfg.turso_url)
        .with_auth_token(&cfg.turso_auth_token)
        .with_client_name("gluebox-stonkwatch-replica")
        .bootstrap_if_empty(true)
        .with_partial_sync_opts_experimental(partial_opts)
        .build()
        .await
        .context("open stonkwatch turso replica")?;

    match db.pull().await {
        Ok(true) => debug!("stonkwatch turso replica: pulled new changes"),
        Ok(false) => debug!("stonkwatch turso replica: already up to date"),
        Err(e) => {
            warn!(error = %e, "stonkwatch turso replica: pull failed, continuing with local state");
        }
    }

    if let Err(e) = db.checkpoint().await {
        warn!(error = %e, "stonkwatch turso replica: checkpoint failed");
    }

    if let Ok(stats) = db.stats().await {
        info!(
            wal_bytes = stats.main_wal_size,
            net_recv = stats.network_received_bytes,
            net_sent = stats.network_sent_bytes,
            "stonkwatch turso replica: sync stats"
        );
    }

    Ok(db)
}
