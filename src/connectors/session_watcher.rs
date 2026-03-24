use std::any::Any;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::pin::Pin;
use std::future::Future;
use std::time::Instant;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use notify::{Watcher, RecursiveMode, Event, EventKind};
use crate::connector::{Connector, ConnectorStatus};

pub struct SessionWatcherConnector {
    config: Mutex<crate::config::WatcherConfig>,
    status: AtomicU8,
    error_msg: Mutex<Option<String>>,
    cancel_token: Mutex<Option<CancellationToken>>,
    on_session_ready: Box<dyn Fn(String) + Send + Sync>,
}

impl SessionWatcherConnector {
    pub fn new(
        config: crate::config::WatcherConfig,
        on_session_ready: Box<dyn Fn(String) + Send + Sync>,
    ) -> Self {
        Self {
            config: Mutex::new(config),
            status: AtomicU8::new(ConnectorStatus::Stopped.as_u8()),
            error_msg: Mutex::new(None),
            cancel_token: Mutex::new(None),
            on_session_ready,
        }
    }
}

impl Connector for SessionWatcherConnector {
    fn name(&self) -> &'static str {
        "watcher"
    }

    fn status(&self) -> ConnectorStatus {
        match self.status.load(Ordering::SeqCst) {
            0 => ConnectorStatus::Running,
            1 => ConnectorStatus::Stopped,
            2 => ConnectorStatus::Suspended,
            _ => {
                let msg = self.error_msg.blocking_lock()
                    .clone()
                    .unwrap_or_default();
                ConnectorStatus::Error(msg)
            }
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn start(&self) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>> {
        Box::pin(async move {
            let config = self.config.lock().await.clone();
            let token = CancellationToken::new();
            let child_token = token.clone();

            let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(256);

            let sessions_dir = config.sessions_dir.clone();
            let debounce_secs = config.debounce_secs;

            std::thread::spawn(move || {
                let rt_tx = tx.clone();
                let mut watcher = match notify::recommended_watcher(
                    move |res: Result<Event, notify::Error>| {
                        if let Ok(event) = res {
                            match event.kind {
                                EventKind::Create(_) | EventKind::Modify(_) => {
                                    for path in &event.paths {
                                        if path.file_name().and_then(|f| f.to_str()) == Some("_summary.md") {
                                            if let Some(session_dir) = path.parent() {
                                                if let Some(session_id) = session_dir.file_name().and_then(|f| f.to_str()) {
                                                    let _ = rt_tx.blocking_send(session_id.to_string());
                                                }
                                            }
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    },
                ) {
                    Ok(w) => w,
                    Err(e) => {
                        tracing::error!("failed to create fs watcher: {e}");
                        return;
                    }
                };

                if let Err(e) = watcher.watch(&sessions_dir, RecursiveMode::Recursive) {
                    tracing::error!("failed to watch {}: {e}", sessions_dir.display());
                    return;
                }

                tracing::info!("session watcher watching {}", sessions_dir.display());
                std::thread::park();
                drop(watcher);
            });

            let on_ready = &self.on_session_ready;

            tokio::spawn(async move {
                let mut debounce_map: HashMap<String, Instant> = HashMap::new();
                let debounce_dur = std::time::Duration::from_secs(debounce_secs);

                loop {
                    tokio::select! {
                        _ = child_token.cancelled() => {
                            break;
                        }
                        Some(session_id) = rx.recv() => {
                            let now = Instant::now();
                            if let Some(last) = debounce_map.get(&session_id) {
                                if now.duration_since(*last) < debounce_dur {
                                    continue;
                                }
                            }
                            debounce_map.insert(session_id.clone(), now);
                            tracing::info!(session_id, "session watcher detected ready session");
                        }
                    }
                }
            });

            *self.cancel_token.lock().await = Some(token);
            self.status.store(ConnectorStatus::Running.as_u8(), Ordering::SeqCst);
            tracing::info!("session watcher started");
            let _ = on_ready;
            Ok(())
        })
    }

    fn stop(&self) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>> {
        Box::pin(async move {
            if let Some(token) = self.cancel_token.lock().await.take() {
                token.cancel();
            }
            self.status.store(ConnectorStatus::Stopped.as_u8(), Ordering::SeqCst);
            tracing::info!("session watcher stopped");
            Ok(())
        })
    }

    fn suspend(&self) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>> {
        Box::pin(async move {
            if let Some(token) = self.cancel_token.lock().await.take() {
                token.cancel();
            }
            self.status.store(ConnectorStatus::Suspended.as_u8(), Ordering::SeqCst);
            Ok(())
        })
    }

    fn resume(&self) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>> {
        self.start()
    }

    fn health_check(&self) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>> {
        Box::pin(async move {
            if self.cancel_token.lock().await.is_some() {
                Ok(())
            } else {
                anyhow::bail!("session watcher not running")
            }
        })
    }
}
