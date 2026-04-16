use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::connector::{Connector, ConnectorStatus};

pub struct ConnectorRegistry {
    connectors: RwLock<HashMap<String, Arc<dyn Connector>>>,
    auto_suspended: RwLock<HashSet<String>>,
}

impl ConnectorRegistry {
    pub fn new() -> Self {
        Self {
            connectors: RwLock::new(HashMap::new()),
            auto_suspended: RwLock::new(HashSet::new()),
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
        let mut suspended = self.auto_suspended.write().await;
        for (name, conn) in lock.iter() {
            if let ConnectorStatus::Running = conn.status() {
                match conn.suspend().await {
                    Ok(()) => {
                        suspended.insert(name.clone());
                    }
                    Err(e) => tracing::error!("failed to suspend {name}: {e}"),
                }
            }
        }
    }

    pub async fn resume_all(&self) {
        let lock = self.connectors.read().await;
        let mut suspended = self.auto_suspended.write().await;
        for name in suspended.iter() {
            if let Some(conn) = lock.get(name) {
                if let Err(e) = conn.resume().await {
                    tracing::error!("failed to resume {name}: {e}");
                }
            }
        }
        suspended.clear();
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicU8, Ordering};
    use crate::connector::{Connector, ConnectorStatus};

    struct TestConnector {
        status: AtomicU8,
        label: &'static str,
    }

    impl TestConnector {
        fn new(label: &'static str) -> Self {
            Self {
                status: AtomicU8::new(1),
                label,
            }
        }
    }

    impl Connector for TestConnector {
        fn name(&self) -> &'static str {
            self.label
        }

        fn status(&self) -> ConnectorStatus {
            match self.status.load(Ordering::SeqCst) {
                0 => ConnectorStatus::Running,
                1 => ConnectorStatus::Stopped,
                2 => ConnectorStatus::Suspended,
                _ => ConnectorStatus::Error("unknown".into()),
            }
        }

        fn as_any(&self) -> &dyn std::any::Any {
            self
        }

        fn start(&self) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>> {
            Box::pin(async {
                self.status.store(0, Ordering::SeqCst);
                Ok(())
            })
        }

        fn stop(&self) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>> {
            Box::pin(async {
                self.status.store(1, Ordering::SeqCst);
                Ok(())
            })
        }

        fn health_check(&self) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>> {
            Box::pin(async { Ok(()) })
        }
    }

    #[tokio::test]
    async fn register_starts_connector() {
        let registry = ConnectorRegistry::new();
        let conn = Arc::new(TestConnector::new("test"));
        registry.register("test".into(), conn.clone()).await.unwrap();
        assert!(matches!(conn.status(), ConnectorStatus::Running));
    }

    #[tokio::test]
    async fn deregister_stops_connector() {
        let registry = ConnectorRegistry::new();
        let conn = Arc::new(TestConnector::new("test"));
        registry.register("test".into(), conn.clone()).await.unwrap();
        let removed = registry.deregister("test").await.unwrap();
        assert!(removed.is_some());
        assert!(matches!(conn.status(), ConnectorStatus::Stopped));
    }

    #[tokio::test]
    async fn toggle_stops_running_connector() {
        let registry = ConnectorRegistry::new();
        let conn = Arc::new(TestConnector::new("test"));
        registry.register("test".into(), conn.clone()).await.unwrap();
        let status = registry.toggle("test").await.unwrap();
        assert!(matches!(status, ConnectorStatus::Stopped));
    }

    #[tokio::test]
    async fn toggle_starts_stopped_connector() {
        let registry = ConnectorRegistry::new();
        let conn = Arc::new(TestConnector::new("test"));
        registry.register("test".into(), conn.clone()).await.unwrap();
        registry.toggle("test").await.unwrap();
        let status = registry.toggle("test").await.unwrap();
        assert!(matches!(status, ConnectorStatus::Running));
    }

    #[tokio::test]
    async fn toggle_nonexistent_returns_error() {
        let registry = ConnectorRegistry::new();
        let result = registry.toggle("nonexistent").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn list_returns_all_connectors() {
        let registry = ConnectorRegistry::new();
        registry.register("alpha".into(), Arc::new(TestConnector::new("alpha"))).await.unwrap();
        registry.register("beta".into(), Arc::new(TestConnector::new("beta"))).await.unwrap();
        registry.register("gamma".into(), Arc::new(TestConnector::new("gamma"))).await.unwrap();
        let list = registry.list().await;
        assert_eq!(list.len(), 3);
        let names: Vec<_> = list.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"alpha"));
        assert!(names.contains(&"beta"));
        assert!(names.contains(&"gamma"));
    }

    #[tokio::test]
    async fn suspend_all_suspends_running() {
        let registry = ConnectorRegistry::new();
        let c1 = Arc::new(TestConnector::new("one"));
        let c2 = Arc::new(TestConnector::new("two"));
        registry.register("one".into(), c1.clone()).await.unwrap();
        registry.register("two".into(), c2.clone()).await.unwrap();
        registry.suspend_all().await;
        assert!(matches!(c1.status(), ConnectorStatus::Stopped));
        assert!(matches!(c2.status(), ConnectorStatus::Stopped));
    }

    #[tokio::test]
    async fn suspend_all_then_resume_all_restarts_connector() {
        let registry = ConnectorRegistry::new();
        let conn = Arc::new(TestConnector::new("test"));
        registry.register("test".into(), conn.clone()).await.unwrap();
        assert!(matches!(conn.status(), ConnectorStatus::Running));

        registry.suspend_all().await;
        assert!(matches!(conn.status(), ConnectorStatus::Stopped));

        registry.resume_all().await;
        assert!(matches!(conn.status(), ConnectorStatus::Running));
    }

    #[tokio::test]
    async fn resume_all_does_not_restart_manually_stopped_connector() {
        let registry = ConnectorRegistry::new();
        let conn = Arc::new(TestConnector::new("test"));
        registry.register("test".into(), conn.clone()).await.unwrap();

        conn.stop().await.unwrap();
        assert!(matches!(conn.status(), ConnectorStatus::Stopped));

        registry.resume_all().await;
        assert!(matches!(conn.status(), ConnectorStatus::Stopped));
    }

    #[tokio::test]
    async fn repeated_suspend_resume_cycles_stay_consistent() {
        let registry = ConnectorRegistry::new();
        let conn = Arc::new(TestConnector::new("test"));
        registry.register("test".into(), conn.clone()).await.unwrap();

        for _ in 0..3 {
            registry.suspend_all().await;
            assert!(matches!(conn.status(), ConnectorStatus::Stopped));
            registry.resume_all().await;
            assert!(matches!(conn.status(), ConnectorStatus::Running));
        }
    }
}
