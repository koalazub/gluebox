use std::any::Any;
use std::future::Future;
use std::pin::Pin;

pub enum ConnectorStatus {
    Running,
    Stopped,
    Suspended,
    Error(String),
}

impl ConnectorStatus {
    pub fn as_u8(&self) -> u8 {
        match self {
            Self::Running => 0,
            Self::Stopped => 1,
            Self::Suspended => 2,
            Self::Error(_) => 3,
        }
    }
}

pub trait Connector: Send + Sync {
    fn name(&self) -> &'static str;
    fn status(&self) -> ConnectorStatus;
    fn as_any(&self) -> &dyn Any;

    fn start(&self) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>>;
    fn stop(&self) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>>;

    fn suspend(&self) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>> {
        self.stop()
    }
    fn resume(&self) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>> {
        self.start()
    }

    fn health_check(&self) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>>;

    fn reconfigure(
        &self,
        _raw_toml: &toml::Value,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<bool>> + Send + '_>> {
        Box::pin(async { Ok(false) })
    }
}
