use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::connectors::matrix::MatrixBot;

const MAX_PER_BUCKET: usize = 50;

pub struct ErrorRollup {
    inner: Mutex<RollupState>,
}

struct BucketEntry {
    samples: Vec<String>,
    truncated_count: usize,
}

struct RollupState {
    buckets: HashMap<String, BucketEntry>,
}

impl ErrorRollup {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(RollupState {
                buckets: HashMap::new(),
            }),
        })
    }

    pub async fn record(&self, category: &'static str, detail: impl Into<String>) {
        let mut state = self.inner.lock().await;
        let entry = state.buckets.entry(category.to_string()).or_insert_with(|| BucketEntry {
            samples: Vec::new(),
            truncated_count: 0,
        });
        if entry.samples.len() < MAX_PER_BUCKET {
            entry.samples.push(detail.into());
        } else {
            entry.truncated_count += 1;
        }
    }

    pub async fn snapshot_to_markdown(&self) -> Option<String> {
        let state = self.inner.lock().await;
        if state.buckets.is_empty() {
            return None;
        }
        let mut out = String::from("### gluebox error rollup (last 5 min)\n\n");
        for (cat, entry) in &state.buckets {
            let count = entry.samples.len() + entry.truncated_count;
            let first = entry.samples.first().cloned().unwrap_or_default();
            let escaped = first.replace('"', "\\\"");
            if entry.truncated_count > 0 {
                out.push_str(&format!(
                    "- **{cat}**: {count} — first: \"{escaped}\" ... ({} more truncated)\n",
                    entry.truncated_count
                ));
            } else {
                out.push_str(&format!("- **{cat}**: {count} — first: \"{escaped}\"\n"));
            }
        }
        Some(out)
    }

    pub async fn clear(&self) {
        self.inner.lock().await.buckets.clear();
    }
}

pub fn spawn_flush_loop(rollup: Arc<ErrorRollup>, bot: Arc<MatrixBot>, room_id: String) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        interval.tick().await;
        loop {
            interval.tick().await;
            if let Some(md) = rollup.snapshot_to_markdown().await {
                match bot.send_markdown_to_room(&room_id, &md).await {
                    Ok(()) => rollup.clear().await,
                    Err(e) => tracing::warn!(error = %e, "error-rollup: failed to flush to matrix"),
                }
            }
        }
    });
}
