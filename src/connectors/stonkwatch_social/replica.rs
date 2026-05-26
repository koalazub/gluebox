use std::time::Duration;

use anyhow::{anyhow, Context};
use tokio::time::timeout;
use tracing::{debug, info, warn};
use turso::sync::{PartialBootstrapStrategy, PartialSyncOpts};

use crate::config::StonkwatchSocialConfig;

const REPLICA_PATH: &str = "/var/lib/gluebox/stonkwatch-replica";

// Turso keeps the local replica as the main db file plus four sidecar files.
// When the local copy is corrupt these must all be removed together so the
// next open re-bootstraps cleanly from the remote.
const REPLICA_SIDECARS: &[&str] = &["", "-wal", "-wal-revert", "-info", "-changes"];

// Only the last 7 days of announcements + their ai_summaries are needed by the
// social posting trigger. Subsetting the bootstrap to those rows keeps the
// replica small and the first pull fast — full DB has much more we never read.
const PARTIAL_BOOTSTRAP_QUERY: &str =
    "SELECT 1 FROM company_announcements WHERE published_at > strftime('%s','now','-7 days') \
     UNION ALL \
     SELECT 1 FROM ai_summaries WHERE created_at > strftime('%s','now','-7 days')";

const PARTIAL_SEGMENT_SIZE: usize = 128 * 1024;

// gluebox is a pure read consumer of this replica — it never writes, so the WAL
// only grows from applying pulled changes. Checkpointing on every open is the
// turso-documented corruption/panic footgun for read-only replicas, so only
// checkpoint once the WAL has actually accumulated past this threshold.
const WAL_CHECKPOINT_THRESHOLD_BYTES: u64 = 32 * 1024 * 1024;

// turso's sync IoWorker uses raw hyper with no HTTP timeout. A stalled
// connection to the remote (TCP black hole, slow DNS, server hang) makes
// build()/pull() never return, wedging the posting loop forever and silently
// halting all social posting. Cap each operation so a single bad request can't
// brick the loop — on timeout we treat it as a transient failure and the next
// cycle retries.
const BOOTSTRAP_TIMEOUT: Duration = Duration::from_secs(180); // initial bootstrap downloads many pages
const PULL_TIMEOUT: Duration = Duration::from_secs(120);
const STATS_TIMEOUT: Duration = Duration::from_secs(10);
const CHECKPOINT_TIMEOUT: Duration = Duration::from_secs(60);

/// Substrings that indicate the *local* replica files are corrupt/truncated
/// (as opposed to a transient network or auth failure). Re-bootstrapping is
/// expensive, so only wipe the local copy on these clearly-local signals.
fn is_local_corruption(err: &anyhow::Error) -> bool {
    let msg = format!("{err:#}").to_lowercase();
    [
        "short read",
        "database disk image is malformed",
        "malformed database",
        "file is not a database",
        "not a database",
        "database corrupt",
        "corrupt page",
        "i/o error: short read",
    ]
    .iter()
    .any(|needle| msg.contains(needle))
}

/// Delete the main replica file and all four turso sidecars so the next open
/// re-bootstraps from the remote. Idempotent — missing files are ignored.
fn reset_replica_files() {
    for suffix in REPLICA_SIDECARS {
        let path = format!("{REPLICA_PATH}{suffix}");
        match std::fs::remove_file(&path) {
            Ok(()) => info!(path, "stonkwatch turso replica: removed corrupt file"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => warn!(path, error = %e, "stonkwatch turso replica: failed to remove file"),
        }
    }
}

pub async fn open_synced_replica(
    cfg: &StonkwatchSocialConfig,
) -> anyhow::Result<turso::sync::Database> {
    match try_open(cfg).await {
        Ok(db) => Ok(db),
        Err(e) if is_local_corruption(&e) => {
            warn!(
                error = format!("{e:#}"),
                "stonkwatch turso replica: local corruption detected, wiping and re-bootstrapping"
            );
            reset_replica_files();
            try_open(cfg)
                .await
                .context("re-bootstrap after corruption reset")
        }
        Err(e) => Err(e),
    }
}

async fn try_open(cfg: &StonkwatchSocialConfig) -> anyhow::Result<turso::sync::Database> {
    let partial_opts = PartialSyncOpts {
        bootstrap_strategy: Some(PartialBootstrapStrategy::Query {
            query: PARTIAL_BOOTSTRAP_QUERY.to_string(),
        }),
        segment_size: PARTIAL_SEGMENT_SIZE,
        prefetch: true,
    };

    let build_fut = turso::sync::Builder::new_remote(REPLICA_PATH)
        .with_remote_url(&cfg.turso_url)
        .with_auth_token(&cfg.turso_auth_token)
        .with_client_name("gluebox-stonkwatch-replica")
        .bootstrap_if_empty(true)
        .with_partial_sync_opts_experimental(partial_opts)
        .build();

    let db = match timeout(BOOTSTRAP_TIMEOUT, build_fut).await {
        Ok(Ok(db)) => db,
        Ok(Err(e)) => return Err(anyhow!(e)).context("open stonkwatch turso replica"),
        Err(_) => {
            return Err(anyhow!(
                "open stonkwatch turso replica: timed out after {}s",
                BOOTSTRAP_TIMEOUT.as_secs()
            ))
        }
    };

    match timeout(PULL_TIMEOUT, db.pull()).await {
        Ok(Ok(true)) => debug!("stonkwatch turso replica: pulled new changes"),
        Ok(Ok(false)) => debug!("stonkwatch turso replica: already up to date"),
        Ok(Err(e)) => {
            let e = anyhow::Error::new(e);
            // A corrupt local copy makes every later query fail too — surface it
            // so the caller wipes and re-bootstraps instead of limping along.
            if is_local_corruption(&e) {
                return Err(e).context("pull failed: local replica corrupt");
            }
            warn!(
                error = format!("{e:#}"),
                "stonkwatch turso replica: pull failed, continuing with local state"
            );
        }
        Err(_) => {
            warn!(
                timeout_secs = PULL_TIMEOUT.as_secs(),
                "stonkwatch turso replica: pull timed out, continuing with local state"
            );
        }
    }

    match timeout(STATS_TIMEOUT, db.stats()).await {
        Ok(Ok(stats)) => {
            if stats.main_wal_size as u64 > WAL_CHECKPOINT_THRESHOLD_BYTES {
                match timeout(CHECKPOINT_TIMEOUT, db.checkpoint()).await {
                    Ok(Ok(())) => debug!(
                        wal_bytes = stats.main_wal_size,
                        "stonkwatch turso replica: checkpointed oversized WAL"
                    ),
                    Ok(Err(e)) => warn!(
                        error = %e,
                        wal_bytes = stats.main_wal_size,
                        "stonkwatch turso replica: checkpoint failed"
                    ),
                    Err(_) => warn!(
                        timeout_secs = CHECKPOINT_TIMEOUT.as_secs(),
                        wal_bytes = stats.main_wal_size,
                        "stonkwatch turso replica: checkpoint timed out"
                    ),
                }
            }
            info!(
                wal_bytes = stats.main_wal_size,
                net_recv = stats.network_received_bytes,
                net_sent = stats.network_sent_bytes,
                "stonkwatch turso replica: sync stats"
            );
        }
        Ok(Err(e)) => debug!(error = %e, "stonkwatch turso replica: stats unavailable"),
        Err(_) => debug!("stonkwatch turso replica: stats timed out"),
    }

    Ok(db)
}
