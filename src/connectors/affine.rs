use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};
use std::any::Any;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};
use std::pin::Pin;
use std::future::Future;
use tokio::sync::Mutex;
use gluebox_core::{Connector, ConnectorStatus};

#[derive(Clone)]
pub struct AffineClient {
    client: Client,
    api_url: String,
    token: String,
    workspace_id: String,
    mcp_url: Option<String>,
    output_dir: PathBuf,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct DocSummary {
    pub id: String,
    pub title: String,
}

impl AffineClient {
    pub fn new(
        api_url: &str,
        token: &str,
        workspace_id: &str,
        mcp_url: Option<String>,
        output_dir: PathBuf,
    ) -> Self {
        Self {
            client: Client::new(),
            api_url: api_url.trim_end_matches('/').to_string(),
            token: token.to_string(),
            workspace_id: workspace_id.to_string(),
            mcp_url,
            output_dir,
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

    /// Create a document in Affine. Tries the MCP endpoint first (supports
    /// markdown import), then falls back to saving locally.
    pub async fn create_document(&self, title: &str, markdown: &str) -> anyhow::Result<String> {
        match self.create_via_mcp(title, markdown).await {
            Ok(doc_id) => Ok(doc_id),
            Err(e) => {
                tracing::warn!("affine MCP create failed ({e}), falling back to local file");
                self.save_local(title, markdown).await
            }
        }
    }

    /// Call the Affine MCP endpoint directly via streamable-HTTP to create a doc.
    async fn create_via_mcp(&self, title: &str, markdown: &str) -> anyhow::Result<String> {
        let mcp_url = self.mcp_url.as_deref()
            .ok_or_else(|| anyhow::anyhow!("no mcp_url configured for this workspace"))?;

        // MCP streamable-HTTP: POST with JSON-RPC
        let rpc_body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "createDocument",
                "arguments": {
                    "title": title,
                    "content": markdown,
                }
            }
        });

        let resp = self.client
            .post(mcp_url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Content-Type", "application/json")
            .json(&rpc_body)
            .send()
            .await?;

        let status = resp.status();
        let body: Value = resp.json().await?;

        if !status.is_success() {
            anyhow::bail!("affine MCP returned {status}: {body}");
        }

        // Extract doc ID from the MCP response
        if let Some(error) = body.get("error") {
            anyhow::bail!("affine MCP error: {error}");
        }

        let result = &body["result"];
        let doc_id = result.get("content")
            .and_then(|c| c.as_array())
            .and_then(|arr| arr.first())
            .and_then(|item| item.get("text"))
            .and_then(|t| t.as_str())
            .unwrap_or("unknown");

        tracing::info!(
            doc_id,
            title,
            workspace = self.workspace_id,
            "created affine doc via MCP"
        );
        Ok(doc_id.to_string())
    }

    async fn save_local(&self, title: &str, markdown: &str) -> anyhow::Result<String> {
        tokio::fs::create_dir_all(&self.output_dir).await?;
        let slug: String = title
            .chars()
            .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
            .collect::<String>()
            .trim_matches('-')
            .to_lowercase();
        let timestamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
        let filename = format!("{timestamp}_{slug}.md");
        let path = self.output_dir.join(&filename);
        let content = if markdown.starts_with("# ") {
            markdown.to_string()
        } else {
            format!("# {title}\n\n{markdown}")
        };
        tokio::fs::write(&path, &content).await?;
        tracing::info!(path = %path.display(), "wrote local fallback doc");
        Ok(path.display().to_string())
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

/// Multi-workspace Affine connector.
pub struct AffineConnector {
    configs: Mutex<HashMap<String, crate::config::AffineConfig>>,
    clients: Mutex<HashMap<String, AffineClient>>,
    status: AtomicU8,
    error_msg: Mutex<Option<String>>,
}

impl AffineConnector {
    pub fn new(configs: HashMap<String, crate::config::AffineConfig>) -> Self {
        Self {
            configs: Mutex::new(configs),
            clients: Mutex::new(HashMap::new()),
            status: AtomicU8::new(ConnectorStatus::Stopped.as_u8()),
            error_msg: Mutex::new(None),
        }
    }

    pub async fn client(&self, workspace: Option<&str>) -> anyhow::Result<AffineClient> {
        let clients = self.clients.lock().await;
        let name = workspace.unwrap_or("default");
        clients
            .get(name)
            .cloned()
            .ok_or_else(|| {
                let available: Vec<_> = clients.keys().collect();
                anyhow::anyhow!(
                    "affine workspace '{}' not found (available: {:?})",
                    name,
                    available
                )
            })
    }

}

fn build_client(cfg: &crate::config::AffineConfig) -> AffineClient {
    AffineClient::new(
        &cfg.api_url,
        &cfg.api_token,
        &cfg.workspace_id,
        cfg.mcp_url.clone(),
        cfg.output_dir.clone(),
    )
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
            let configs = self.configs.lock().await;
            let mut clients = self.clients.lock().await;
            clients.clear();
            for (name, cfg) in configs.iter() {
                tracing::info!(workspace = name, "affine workspace registered");
                clients.insert(name.clone(), build_client(cfg));
            }
            drop(configs);
            drop(clients);
            self.status.store(ConnectorStatus::Running.as_u8(), Ordering::SeqCst);
            Ok(())
        })
    }

    fn stop(&self) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>> {
        Box::pin(async move {
            self.clients.lock().await.clear();
            self.status.store(ConnectorStatus::Stopped.as_u8(), Ordering::SeqCst);
            Ok(())
        })
    }

    fn health_check(&self) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>> {
        Box::pin(async move {
            if !self.clients.lock().await.is_empty() {
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
        let result = raw_toml.clone().try_into::<HashMap<String, crate::config::AffineConfig>>();
        Box::pin(async move {
            let new_configs = result.map_err(|e| anyhow::anyhow!("invalid affine config: {e}"))?;
            let mut clients = self.clients.lock().await;
            clients.clear();
            for (name, cfg) in &new_configs {
                clients.insert(name.clone(), build_client(cfg));
            }
            *self.configs.lock().await = new_configs;
            Ok(true)
        })
    }
}
