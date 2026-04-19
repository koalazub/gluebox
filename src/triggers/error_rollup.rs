use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::connectors::matrix::MatrixBot;

pub struct ErrorRollup {
    inner: Mutex<RollupState>,
}

struct RollupState {
    buckets: HashMap<String, Vec<String>>,
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
        state.buckets.entry(category.to_string()).or_default().push(detail.into());
    }

    pub async fn drain_to_markdown(&self) -> Option<String> {
        let mut state = self.inner.lock().await;
        if state.buckets.is_empty() {
            return None;
        }
        let mut out = String::from("### gluebox error rollup (last 5 min)\n\n");
        for (cat, details) in state.buckets.drain() {
            let count = details.len();
            let first = details.first().cloned().unwrap_or_default();
            out.push_str(&format!("- **{cat}**: {count} — first: `{first}`\n"));
        }
        Some(out)
    }
}

pub fn spawn_flush_loop(rollup: Arc<ErrorRollup>, bot: Arc<MatrixBot>, room_id: String) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if let Some(md) = rollup.drain_to_markdown().await {
                if let Err(e) = bot.send_markdown_to_room(&room_id, &md).await {
                    tracing::warn!(error = %e, "error-rollup: failed to flush to matrix");
                }
            }
        }
    });
}
