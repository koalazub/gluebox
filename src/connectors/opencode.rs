use reqwest;
use serde::Deserialize;
use serde_json::json;
use std::any::Any;
use std::sync::atomic::{AtomicU8, Ordering};
use std::pin::Pin;
use std::future::Future;
use tokio::sync::Mutex;
use gluebox_core::{Connector, ConnectorStatus};

#[derive(Clone)]
pub struct OpenCodeClient {
    client: reqwest::Client,
    api_key: String,
    base_url: String,
    model: String,
}

#[derive(Debug, Deserialize)]
pub struct ChatCompletionResponse {
    pub choices: Vec<Choice>,
}

#[derive(Debug, Deserialize)]
pub struct Choice {
    pub message: Message,
    #[allow(dead_code)]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Message {
    pub content: Option<String>,
    pub reasoning_content: Option<String>,
    #[allow(dead_code)]
    pub role: String,
}

impl OpenCodeClient {
    pub fn new(api_key: &str) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key: api_key.to_string(),
            base_url: "https://openrouter.ai/api/v1".to_string(),
            model: "google/gemini-2.5-flash-lite".to_string(),
        }
    }

    pub async fn chat(&self, system: &str, user: &str, max_tokens: u32) -> anyhow::Result<String> {
        let url = format!("{}/chat/completions", self.base_url);
        
        let payload = json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": user}
            ],
            "max_tokens": max_tokens,
            "temperature": 0.7,
        });

        let resp = self.client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await?;
            if status.as_u16() == 429 {
                anyhow::bail!("Rate limited - try again in a minute");
            }
            anyhow::bail!("AI error ({}): {}", status, text);
        }

        let text = resp.text().await?;
        let data: ChatCompletionResponse = serde_json::from_str(&text)
            .map_err(|e| anyhow::anyhow!("Failed to parse AI response: {e}\nRaw: {}", &text[..text.len().min(200)]))?;
        
        let content = data.choices
            .first()
            .and_then(|c| c.message.content.as_ref())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .or_else(|| {
                data.choices
                    .first()
                    .and_then(|c| c.message.reasoning_content.as_ref())
                    .map(|s| s.trim().to_string())
            })
            .unwrap_or_else(|| "(no response)".to_string());

        Ok(content)
    }

    /// Extract discrete feedback items from a message, cluster similar ones,
    /// and categorise each cluster. Returns an array ready to file as tickets.
    pub async fn extract_and_cluster_feedback(
        &self,
        message: &str,
    ) -> anyhow::Result<Vec<FeedbackCluster>> {
        let system = r#"You are a product feedback analyst. Extract discrete feedback items from the user message, cluster similar ones together, and categorise each cluster.

Output ONLY a JSON array (no markdown, no code fences). Each element must have:
- "title": short actionable title under 80 chars, suitable for a Linear issue
- "description": detailed description with what the user wants and why, suitable for a Linear issue body
- "category": exactly one of: "bug" | "feature" | "ux" | "performance" | "docs" | "other"
- "items": array of verbatim or closely paraphrased individual feedback points in this cluster

Rules:
- Merge feedback items that describe the same root problem into one cluster
- Do NOT merge unrelated feedback just to reduce count
- If there is only one distinct topic, return a single-element array
- Ignore greetings, meta-commentary, and off-topic content"#;

        let response = self.chat(system, message, 3000).await?;
        let cleaned = response
            .trim()
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim();

        let clusters: Vec<FeedbackCluster> = serde_json::from_str(cleaned)
            .map_err(|e| anyhow::anyhow!("failed to parse feedback clusters: {e}\nRaw: {}", &cleaned[..cleaned.len().min(400)]))?;

        Ok(clusters)
    }

    /// Given a new cluster title+description and a list of existing issue
    /// summaries, return the ID of the matching existing issue if this is a
    /// duplicate, or None if it is genuinely new.
    pub async fn find_duplicate_issue(
        &self,
        cluster: &FeedbackCluster,
        existing: &[ExistingIssueSummary],
    ) -> anyhow::Result<Option<String>> {
        if existing.is_empty() {
            return Ok(None);
        }

        let existing_json = serde_json::to_string(existing)?;

        let user_prompt = format!(
            "New feedback cluster:\nTitle: {}\nDescription: {}\nCategory: {}\n\nExisting issues:\n{}",
            cluster.title, cluster.description, cluster.category, existing_json
        );

        let system = r#"You are deduplicating product feedback against existing Linear issues.

Given a new feedback cluster and a list of existing issues, determine if the new cluster describes the SAME root problem as any existing issue.

Criteria for a duplicate:
- Same underlying user pain point or request (even if described differently)
- Same category
- A comment on the existing issue would be more appropriate than a new ticket

Output ONLY a JSON object (no markdown):
- {"duplicate": true, "id": "<issue_id>"} if it matches an existing issue
- {"duplicate": false} if it is genuinely new

Be conservative: only mark as duplicate when confident. Different symptoms of the same system can be separate issues."#;

        let response = self.chat(system, &user_prompt, 300).await?;
        let cleaned = response
            .trim()
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim();

        let parsed: serde_json::Value = serde_json::from_str(cleaned)
            .unwrap_or_else(|_| serde_json::json!({"duplicate": false}));

        if parsed["duplicate"].as_bool().unwrap_or(false) {
            Ok(parsed["id"].as_str().map(|s| s.to_string()))
        } else {
            Ok(None)
        }
    }
}

/// Lightweight summary of an existing Linear issue used for deduplication.
#[derive(Debug, serde::Serialize)]
pub struct ExistingIssueSummary {
    pub id: String,
    pub title: String,
    pub category: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct FeedbackCluster {
    pub title: String,
    pub description: String,
    pub category: String,
    pub items: Vec<String>,
}

pub struct OpenCodeConnector {
    config: Mutex<crate::config::OpenCodeConfig>,
    client: Mutex<Option<OpenCodeClient>>,
    status: AtomicU8,
    error_msg: Mutex<Option<String>>,
}

impl OpenCodeConnector {
    pub fn new(config: crate::config::OpenCodeConfig) -> Self {
        Self {
            config: Mutex::new(config),
            client: Mutex::new(None),
            status: AtomicU8::new(ConnectorStatus::Stopped.as_u8()),
            error_msg: Mutex::new(None),
        }
    }

    pub async fn client(&self) -> anyhow::Result<OpenCodeClient> {
        self.client
            .lock()
            .await
            .clone()
            .ok_or_else(|| anyhow::anyhow!("opencode connector not running"))
    }
}

impl Connector for OpenCodeConnector {
    fn name(&self) -> &'static str {
        "opencode"
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
            let new_client = OpenCodeClient::new(&config.api_key);
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
                anyhow::bail!("opencode connector not running")
            }
        })
    }

    fn reconfigure(
        &self,
        raw_toml: &toml::Value,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<bool>> + Send + '_>> {
        let result = raw_toml.clone().try_into::<crate::config::OpenCodeConfig>();
        Box::pin(async move {
            let new_config = result.map_err(|e| anyhow::anyhow!("invalid opencode config: {e}"))?;
            *self.config.lock().await = new_config;
            Ok(true)
        })
    }
}
