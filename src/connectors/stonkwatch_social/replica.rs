use anyhow::Context;
use tracing::{debug, warn};

use crate::config::StonkwatchSocialConfig;

const REPLICA_PATH: &str = "/var/lib/gluebox/stonkwatch-replica";

/// Opens the embedded Turso replica for the stonkwatch master DB and pulls the
/// latest changes before handing it back. Every read path goes through here so
/// queries see fresh rows instead of whatever the replica happened to have on
/// disk from the last time this process restarted.
///
/// `bootstrap_if_empty(true)` covers the cold-start case where the replica file
/// doesn't exist yet. The subsequent explicit `pull()` is what closes the drift
/// window for a replica that was initialised days ago and hasn't been touched
/// since.
pub async fn open_synced_replica(
    cfg: &StonkwatchSocialConfig,
) -> anyhow::Result<turso::sync::Database> {
    let db = turso::sync::Builder::new_remote(REPLICA_PATH)
        .with_remote_url(&cfg.turso_url)
        .with_auth_token(&cfg.turso_auth_token)
        .bootstrap_if_empty(true)
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

    Ok(db)
}
