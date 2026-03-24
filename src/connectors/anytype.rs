use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::any::Any;
use std::sync::atomic::{AtomicU8, Ordering};
use std::pin::Pin;
use std::future::Future;
use tokio::sync::Mutex;
use crate::connector::{Connector, ConnectorStatus};

#[derive(Clone)]
pub struct AnytypeClient {
    client: Client,
    base_url: String,
    api_key: String,
    space_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnytypeObject {
    pub id: String,
    pub name: Option<String>,
    pub description: Option<String>,
    #[serde(rename = "type")]
    pub object_type: Option<Value>,
    pub properties: Option<Value>,
}

impl AnytypeClient {
    pub fn new(base_url: &str, api_key: &str, space_id: &str) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.to_string(),
            space_id: space_id.to_string(),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}/v1/spaces/{}{}", self.base_url, self.space_id, path)
    }

    async fn request(&self, method: reqwest::Method, path: &str, body: Option<Value>) -> anyhow::Result<Value> {
        let mut req = self.client
            .request(method, self.url(path))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Anytype-Version", "2025-11-08")
            .header("Content-Type", "application/json");
        if let Some(b) = body {
            req = req.json(&b);
        }
        let resp = req.send().await?.error_for_status()?;
        let val: Value = resp.json().await?;
        Ok(val)
    }

    pub async fn create_object(&self, type_key: &str, name: &str, description: &str, body_markdown: Option<&str>) -> anyhow::Result<AnytypeObject> {
        let mut payload = json!({
            "type_key": type_key,
            "name": name,
            "description": description,
        });
        if let Some(md) = body_markdown {
            payload["body"] = json!(md);
        }
        let resp = self.request(reqwest::Method::POST, "/objects", Some(payload)).await?;
        let obj: AnytypeObject = serde_json::from_value(resp["object"].clone())?;
        Ok(obj)
    }

    pub async fn update_object(&self, object_id: &str, updates: Value) -> anyhow::Result<AnytypeObject> {
        let resp = self.request(
            reqwest::Method::PATCH,
            &format!("/objects/{object_id}"),
            Some(updates),
        ).await?;
        let obj: AnytypeObject = serde_json::from_value(resp["object"].clone())?;
        Ok(obj)
    }

    #[allow(dead_code)]
    pub async fn get_object(&self, object_id: &str) -> anyhow::Result<AnytypeObject> {
        let resp = self.request(
            reqwest::Method::GET,
            &format!("/objects/{object_id}"),
            None,
        ).await?;
        let obj: AnytypeObject = serde_json::from_value(resp["object"].clone())?;
        Ok(obj)
    }

    #[allow(dead_code)]
    pub async fn search_objects(&self, query: &str) -> anyhow::Result<Vec<AnytypeObject>> {
        let resp = self.request(
            reqwest::Method::GET,
            &format!("/objects?query={}", urlencoding::encode(query)),
            None,
        ).await?;
        let objects: Vec<AnytypeObject> = serde_json::from_value(
            resp["objects"].clone()
        ).unwrap_or_default();
        Ok(objects)
    }

    pub async fn list_types(&self) -> anyhow::Result<Value> {
        self.request(reqwest::Method::GET, "/types", None).await
    }

    pub async fn type_exists(&self, type_key: &str) -> anyhow::Result<bool> {
        let types = self.list_types().await?;
        let exists = types["data"]
            .as_array()
            .map(|arr| arr.iter().any(|t| t["key"].as_str() == Some(type_key)))
            .unwrap_or(false);
        Ok(exists)
    }

    pub async fn create_type(&self, key: &str, name: &str, plural_name: &str, icon_name: &str, icon_color: &str) -> anyhow::Result<Value> {
        let payload = json!({
            "key": key,
            "name": name,
            "plural_name": plural_name,
            "icon": {
                "format": "icon",
                "name": icon_name,
                "color": icon_color
            },
            "layout": "basic"
        });
        self.request(reqwest::Method::POST, "/types", Some(payload)).await
    }

    pub async fn ensure_types(&self) -> anyhow::Result<()> {
        let types = vec![
            ("spec", "Spec", "Specs", "document-text", "blue"),
            ("contract", "Contract", "Contracts", "document-lock", "green"),
            ("decision", "Decision", "Decisions", "git-branch", "orange"),
        ];

        for (key, name, plural, icon, color) in types {
            match self.type_exists(key).await {
                Ok(true) => {
                    tracing::debug!(type_key = key, "object type already exists");
                }
                Ok(false) => {
                    tracing::info!(type_key = key, "creating object type in anytype");
                    match self.create_type(key, name, plural, icon, color).await {
                        Ok(_) => tracing::info!(type_key = key, "object type created"),
                        Err(e) => {
                            tracing::warn!(type_key = key, error = %e, "failed to create type, may already exist");
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(type_key = key, error = %e, "failed to check if type exists");
                }
            }
        }
        Ok(())
    }
}

pub struct AnytypeConnector {
    config: Mutex<crate::config::AnytypeConfig>,
    client: Mutex<Option<AnytypeClient>>,
    status: AtomicU8,
    error_msg: Mutex<Option<String>>,
}

impl AnytypeConnector {
    pub fn new(config: crate::config::AnytypeConfig) -> Self {
        Self {
            config: Mutex::new(config),
            client: Mutex::new(None),
            status: AtomicU8::new(ConnectorStatus::Stopped.as_u8()),
            error_msg: Mutex::new(None),
        }
    }

    pub async fn client(&self) -> anyhow::Result<AnytypeClient> {
        self.client
            .lock()
            .await
            .clone()
            .ok_or_else(|| anyhow::anyhow!("anytype connector not running"))
    }
}

impl Connector for AnytypeConnector {
    fn name(&self) -> &'static str {
        "anytype"
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
            let new_client = AnytypeClient::new(&config.api_url, &config.api_key, &config.space_id);
            drop(config);
            new_client.ensure_types().await?;
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
                anyhow::bail!("anytype connector not running")
            }
        })
    }

    fn reconfigure(
        &self,
        raw_toml: &toml::Value,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<bool>> + Send + '_>> {
        let result = raw_toml.clone().try_into::<crate::config::AnytypeConfig>();
        Box::pin(async move {
            let new_config = result.map_err(|e| anyhow::anyhow!("invalid anytype config: {e}"))?;
            *self.config.lock().await = new_config;
            Ok(true)
        })
    }
}
