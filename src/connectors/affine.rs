use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};
use std::any::Any;
use std::sync::atomic::{AtomicU8, Ordering};
use std::pin::Pin;
use std::future::Future;
use tokio::sync::Mutex;
use crate::connector::{Connector, ConnectorStatus};

#[derive(Clone)]
pub struct AffineClient {
    client: Client,
    api_url: String,
    token: String,
    workspace_id: String,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct DocSummary {
    pub id: String,
    pub title: String,
}

impl AffineClient {
    pub fn new(api_url: &str, token: &str, workspace_id: &str) -> Self {
        Self {
            client: Client::new(),
            api_url: api_url.trim_end_matches('/').to_string(),
            token: token.to_string(),
            workspace_id: workspace_id.to_string(),
        }
    }

    async fn graphql(&self, query: &str, variables: Option<Value>) -> anyhow::Result<Value> {
        let body = json!({
            "query": query,
            "variables": variables.unwrap_or(json!({})),
        });
        let resp = self.client
            .post(&self.api_url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?
            .error_for_status()?
            .json::<Value>()
            .await?;
        if let Some(errors) = resp.get("errors") {
            anyhow::bail!("affine graphql errors: {errors}");
        }
        Ok(resp)
    }

    pub async fn create_document(&self, title: &str, markdown: &str) -> anyhow::Result<String> {
        let query = r#"
            mutation($input: CreateDocInput!) {
                createDoc(input: $input) { id }
            }
        "#;
        let vars = json!({
            "input": {
                "workspaceId": self.workspace_id,
                "title": title,
                "markdown": markdown,
            }
        });
        let resp = self.graphql(query, Some(vars)).await?;
        let doc_id = resp["data"]["createDoc"]["id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing doc id in affine response"))?
            .to_string();
        Ok(doc_id)
    }

    #[allow(dead_code)]
    pub async fn search_documents(&self, query_text: &str) -> anyhow::Result<Vec<DocSummary>> {
        let query = r#"
            query($workspaceId: String!, $query: String!) {
                searchDocs(workspaceId: $workspaceId, query: $query) {
                    id
                    title
                }
            }
        "#;
        let vars = json!({
            "workspaceId": self.workspace_id,
            "query": query_text,
        });
        let resp = self.graphql(query, Some(vars)).await?;
        let docs: Vec<DocSummary> = serde_json::from_value(
            resp["data"]["searchDocs"].clone()
        ).unwrap_or_default();
        Ok(docs)
    }
}

pub struct AffineConnector {
    config: Mutex<crate::config::AffineConfig>,
    client: Mutex<Option<AffineClient>>,
    status: AtomicU8,
    error_msg: Mutex<Option<String>>,
}

impl AffineConnector {
    pub fn new(config: crate::config::AffineConfig) -> Self {
        Self {
            config: Mutex::new(config),
            client: Mutex::new(None),
            status: AtomicU8::new(ConnectorStatus::Stopped.as_u8()),
            error_msg: Mutex::new(None),
        }
    }

    pub async fn client(&self) -> anyhow::Result<AffineClient> {
        self.client
            .lock()
            .await
            .clone()
            .ok_or_else(|| anyhow::anyhow!("affine connector not running"))
    }
}

impl Connector for AffineConnector {
    fn name(&self) -> &'static str {
        "affine"
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
            let config = self.config.lock().await;
            let new_client = AffineClient::new(&config.api_url, &config.api_token, &config.workspace_id);
            drop(config);
            *self.client.lock().await = Some(new_client);
            self.status.store(ConnectorStatus::Running.as_u8(), Ordering::SeqCst);
            Ok(())
        })
    }

    fn stop(&self) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>> {
        Box::pin(async move {
            *self.client.lock().await = None;
            self.status.store(ConnectorStatus::Stopped.as_u8(), Ordering::SeqCst);
            Ok(())
        })
    }

    fn health_check(&self) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>> {
        Box::pin(async move {
            if self.client.lock().await.is_some() {
                Ok(())
            } else {
                anyhow::bail!("affine connector not running")
            }
        })
    }

    fn reconfigure(
        &self,
        raw_toml: &toml::Value,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<bool>> + Send + '_>> {
        let result = raw_toml.clone().try_into::<crate::config::AffineConfig>();
        Box::pin(async move {
            let new_config = result.map_err(|e| anyhow::anyhow!("invalid affine config: {e}"))?;
            *self.config.lock().await = new_config;
            Ok(true)
        })
    }
}
