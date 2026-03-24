use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::connector::{Connector, ConnectorStatus};

pub struct ConnectorRegistry {
    connectors: RwLock<HashMap<String, Arc<dyn Connector>>>,
}

impl ConnectorRegistry {
    pub fn new() -> Self {
        Self {
            connectors: RwLock::new(HashMap::new()),
        }
    }

    pub async fn register(&self, name: String, connector: Arc<dyn Connector>) -> anyhow::Result<()> {
        connector.start().await?;
        self.connectors.write().await.insert(name, connector);
        Ok(())
    }

    pub async fn deregister(&self, name: &str) -> anyhow::Result<Option<Arc<dyn Connector>>> {
        let conn = self.connectors.write().await.remove(name);
        if let Some(ref c) = conn {
            c.stop().await?;
        }
        Ok(conn)
    }

    pub async fn get_dyn(&self, name: &str) -> Option<Arc<dyn Connector>> {
        self.connectors.read().await.get(name).cloned()
    }

    pub async fn toggle(&self, name: &str) -> anyhow::Result<ConnectorStatus> {
        let conn = self
            .connectors
            .read()
            .await
            .get(name)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("connector not found: {name}"))?;

        match conn.status() {
            ConnectorStatus::Running => {
                conn.stop().await?;
                Ok(conn.status())
            }
            ConnectorStatus::Stopped | ConnectorStatus::Suspended | ConnectorStatus::Error(_) => {
                conn.start().await?;
                Ok(conn.status())
            }
        }
    }

    pub async fn suspend_all(&self) {
        let lock = self.connectors.read().await;
        for (name, conn) in lock.iter() {
            if let ConnectorStatus::Running = conn.status() {
                if let Err(e) = conn.suspend().await {
                    tracing::error!("failed to suspend {name}: {e}");
                }
            }
        }
    }

    pub async fn resume_all(&self) {
        let lock = self.connectors.read().await;
        for (name, conn) in lock.iter() {
            if let ConnectorStatus::Suspended = conn.status() {
                if let Err(e) = conn.resume().await {
                    tracing::error!("failed to resume {name}: {e}");
                }
            }
        }
    }

    pub async fn stop_all(&self) {
        let lock = self.connectors.read().await;
        for (name, conn) in lock.iter() {
            match conn.status() {
                ConnectorStatus::Stopped => {}
                _ => {
                    if let Err(e) = conn.stop().await {
                        tracing::error!("failed to stop {name}: {e}");
                    }
                }
            }
        }
    }

    pub async fn list(&self) -> Vec<(String, ConnectorStatus)> {
        let lock = self.connectors.read().await;
        lock.iter()
            .map(|(name, conn)| (name.clone(), conn.status()))
            .collect()
    }

    pub async fn names(&self) -> Vec<String> {
        self.connectors.read().await.keys().cloned().collect()
    }
}
