use reqwest::Client;
use serde::Deserialize;
use serde_json::{Value, json};
use std::any::Any;
use std::sync::atomic::{AtomicU8, Ordering};
use std::pin::Pin;
use std::future::Future;
use tokio::sync::Mutex;
use crate::connector::{Connector, ConnectorStatus};

#[derive(Clone)]
pub struct GithubClient {
    client: Client,
    token: String,
    repo: String,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct GithubIssue {
    pub number: i64,
    pub title: String,
    pub body: Option<String>,
    pub html_url: String,
    pub state: String,
}

impl GithubClient {
    pub fn new(token: &str, repo: &str) -> Self {
        Self {
            client: Client::new(),
            token: token.to_string(),
            repo: repo.to_string(),
        }
    }

    #[allow(dead_code)]
    pub fn repo(&self) -> &str {
        &self.repo
    }

    async fn api(&self, method: reqwest::Method, path: &str, body: Option<Value>) -> anyhow::Result<Value> {
        let url = format!("https://api.github.com/repos/{}/{}", self.repo, path);
        let mut req = self.client
            .request(method, &url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("User-Agent", "gluebox/1.0");

        if let Some(b) = body {
            req = req.json(&b);
        }

        Ok(req.send().await?.error_for_status()?.json::<Value>().await?)
    }

    pub async fn create_issue(&self, title: &str, body: &str, labels: &[&str]) -> anyhow::Result<GithubIssue> {
        let payload = json!({ "title": title, "body": body, "labels": labels });
        let resp = self.api(reqwest::Method::POST, "issues", Some(payload)).await?;
        Ok(serde_json::from_value(resp)?)
    }

    #[allow(dead_code)]
    pub async fn create_comment(&self, number: i64, body: &str) -> anyhow::Result<()> {
        self.api(
            reqwest::Method::POST,
            &format!("issues/{number}/comments"),
            Some(json!({ "body": body })),
        ).await?;
        Ok(())
    }
}

pub struct GithubConnector {
    config: Mutex<crate::config::GithubConfig>,
    client: Mutex<Option<GithubClient>>,
    status: AtomicU8,
    error_msg: Mutex<Option<String>>,
}

impl GithubConnector {
    pub fn new(config: crate::config::GithubConfig) -> Self {
        Self {
            config: Mutex::new(config),
            client: Mutex::new(None),
            status: AtomicU8::new(ConnectorStatus::Stopped.as_u8()),
            error_msg: Mutex::new(None),
        }
    }

    pub async fn client(&self) -> anyhow::Result<GithubClient> {
        self.client
            .lock()
            .await
            .clone()
            .ok_or_else(|| anyhow::anyhow!("github connector not running"))
    }
}

impl Connector for GithubConnector {
    fn name(&self) -> &'static str {
        "github"
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
            let new_client = GithubClient::new(&config.token, &config.repo);
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
                anyhow::bail!("github connector not running")
            }
        })
    }

    fn reconfigure(
        &self,
        raw_toml: &toml::Value,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<bool>> + Send + '_>> {
        let result = raw_toml.clone().try_into::<crate::config::GithubConfig>();
        Box::pin(async move {
            let new_config = result.map_err(|e| anyhow::anyhow!("invalid github config: {e}"))?;
            *self.config.lock().await = new_config;
            Ok(true)
        })
    }
}
