use serde::{Deserialize, Serialize};
use std::any::Any;
use std::sync::atomic::{AtomicU8, Ordering};
use std::pin::Pin;
use std::future::Future;
use crate::connector::{Connector, ConnectorStatus};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookPayload {
    pub event: String,
    pub payload: DocumentPayload,
    #[serde(rename = "createdAt")]
    pub created_at: String,
    #[serde(rename = "webhookEndpoint")]
    pub webhook_endpoint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentPayload {
    pub id: i64,
    #[serde(rename = "externalId")]
    pub external_id: Option<String>,
    pub title: String,
    pub status: String,
    #[serde(rename = "completedAt")]
    pub completed_at: Option<String>,
    #[serde(rename = "Recipient")]
    pub recipients: Option<Vec<Recipient>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recipient {
    pub email: String,
    pub name: String,
    pub role: String,
    #[serde(rename = "signingStatus")]
    pub signing_status: String,
    #[serde(rename = "rejectionReason")]
    pub rejection_reason: Option<String>,
}

pub struct DocumensoConnector {
    status: AtomicU8,
}

impl DocumensoConnector {
    pub fn new() -> Self {
        Self {
            status: AtomicU8::new(ConnectorStatus::Stopped.as_u8()),
        }
    }
}

impl Connector for DocumensoConnector {
    fn name(&self) -> &'static str {
        "documenso"
    }

    fn status(&self) -> ConnectorStatus {
        match self.status.load(Ordering::SeqCst) {
            0 => ConnectorStatus::Running,
            1 => ConnectorStatus::Stopped,
            2 => ConnectorStatus::Suspended,
            _ => ConnectorStatus::Error(String::new()),
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn start(&self) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>> {
        Box::pin(async move {
            self.status.store(ConnectorStatus::Running.as_u8(), Ordering::SeqCst);
            Ok(())
        })
    }

    fn stop(&self) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>> {
        Box::pin(async move {
            self.status.store(ConnectorStatus::Stopped.as_u8(), Ordering::SeqCst);
            Ok(())
        })
    }

    fn health_check(&self) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>> {
        Box::pin(async move {
            Ok(())
        })
    }
}
