use serde::{Deserialize, Serialize};

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
