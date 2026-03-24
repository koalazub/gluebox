use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Clone)]
pub struct LinearClient {
    client: Client,
    api_key: String,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct Issue {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub url: String,
    pub state: Option<IssueState>,
    pub priority: Option<f64>,
    pub labels: Option<Labels>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct IssueState {
    pub name: String,
    #[serde(rename = "type")]
    pub state_type: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct Labels {
    pub nodes: Vec<Label>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct Label {
    pub name: String,
}

impl LinearClient {
    pub fn new(api_key: &str) -> Self {
        Self {
            client: Client::new(),
            api_key: api_key.to_string(),
        }
    }

    pub(crate) async fn graphql(&self, query: &str, variables: Option<Value>) -> anyhow::Result<Value> {
        let body = json!({
            "query": query,
            "variables": variables.unwrap_or(json!({})),
        });
        let resp = self.client
            .post("https://api.linear.app/graphql")
            .header("Authorization", &self.api_key)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?
            .error_for_status()?
            .json::<Value>()
            .await?;
        if let Some(errors) = resp.get("errors") {
            anyhow::bail!("linear graphql errors: {errors}");
        }
        Ok(resp)
    }

    #[allow(dead_code)]
    pub async fn get_issue(&self, issue_id: &str) -> anyhow::Result<Issue> {
        let query = r#"
            query($id: String!) {
                issue(id: $id) {
                    id title description url priority
                    state { name type }
                    labels { nodes { name } }
                }
            }
        "#;
        let resp = self.graphql(query, Some(json!({ "id": issue_id }))).await?;
        let issue: Issue = serde_json::from_value(
            resp["data"]["issue"].clone()
        )?;
        Ok(issue)
    }

    pub async fn add_comment(&self, issue_id: &str, body: &str) -> anyhow::Result<()> {
        let query = r#"
            mutation($input: CommentCreateInput!) {
                commentCreate(input: $input) { success }
            }
        "#;
        let vars = json!({
            "input": {
                "issueId": issue_id,
                "body": body,
            }
        });
        self.graphql(query, Some(vars)).await?;
        Ok(())
    }

    pub async fn update_issue_description(&self, issue_id: &str, description: &str) -> anyhow::Result<()> {
        let query = r#"
            mutation($id: String!, $input: IssueUpdateInput!) {
                issueUpdate(id: $id, input: $input) { success }
            }
        "#;
        let vars = json!({
            "id": issue_id,
            "input": { "description": description }
        });
        self.graphql(query, Some(vars)).await?;
        Ok(())
    }

    #[allow(dead_code)]
    pub async fn get_issues_with_label(&self, label_name: &str, team_id: Option<&str>) -> anyhow::Result<Vec<Issue>> {
        let query = r#"
            query($filter: IssueFilter) {
                issues(filter: $filter) {
                    nodes {
                        id title description url priority
                        state { name type }
                        labels { nodes { name } }
                    }
                }
            }
        "#;
        let mut filter = json!({
            "labels": { "name": { "eq": label_name } }
        });
        if let Some(tid) = team_id {
            filter["team"] = json!({ "id": { "eq": tid } });
        }
        let resp = self.graphql(query, Some(json!({ "filter": filter }))).await?;
        let issues: Vec<Issue> = serde_json::from_value(
            resp["data"]["issues"]["nodes"].clone()
        )?;
        Ok(issues)
    }

    #[allow(dead_code)]
    pub async fn set_issue_state(&self, issue_id: &str, state_id: &str) -> anyhow::Result<()> {
        let query = r#"
            mutation($id: String!, $input: IssueUpdateInput!) {
                issueUpdate(id: $id, input: $input) { success }
            }
        "#;
        let vars = json!({
            "id": issue_id,
            "input": { "stateId": state_id }
        });
        self.graphql(query, Some(vars)).await?;
        Ok(())
    }

    /// Find an existing label by name (case-insensitive), or create it.
    /// Returns the label ID.
    pub async fn get_or_create_label(
        &self,
        team_id: &str,
        name: &str,
        color: &str,
    ) -> anyhow::Result<String> {
        // Fetch existing labels for the team
        let query = r#"
            query($teamId: String!) {
                team(id: $teamId) { labels { nodes { id name } } }
            }
        "#;
        let resp = self.graphql(query, Some(json!({ "teamId": team_id }))).await?;
        let labels = resp["data"]["team"]["labels"]["nodes"]
            .as_array()
            .cloned()
            .unwrap_or_default();

        for label in &labels {
            if label["name"]
                .as_str()
                .map(|s| s.eq_ignore_ascii_case(name))
                .unwrap_or(false)
            {
                return Ok(label["id"].as_str().unwrap_or_default().to_string());
            }
        }

        // Create the label if not found
        let create = r#"
            mutation($input: IssueLabelCreateInput!) {
                issueLabelCreate(input: $input) { issueLabel { id } }
            }
        "#;
        let resp = self
            .graphql(
                create,
                Some(json!({
                    "input": { "teamId": team_id, "name": name, "color": color }
                })),
            )
            .await?;
        let id = resp["data"]["issueLabelCreate"]["issueLabel"]["id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("failed to create label '{name}'"))?
            .to_string();
        Ok(id)
    }

    /// Create an issue tagged with one label. Returns the full response Value.
    pub async fn create_issue_with_label(
        &self,
        title: &str,
        description: &str,
        team_id: &str,
        label_id: &str,
    ) -> anyhow::Result<Value> {
        let query = r#"
            mutation($input: IssueCreateInput!) {
                issueCreate(input: $input) {
                    success
                    issue { id title url }
                }
            }
        "#;
        let vars = json!({
            "input": {
                "title": title,
                "description": description,
                "teamId": team_id,
                "labelIds": [label_id],
            }
        });
        self.graphql(query, Some(vars)).await
    }

    /// Add a comment to an existing issue noting a duplicate piece of feedback.
    pub async fn add_feedback_comment(
        &self,
        issue_id: &str,
        feedback_items: &[String],
        source_note: &str,
    ) -> anyhow::Result<()> {
        let items = feedback_items
            .iter()
            .map(|i| format!("- {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let body = format!(
            "**Additional feedback received** ({source_note}):\n\n{items}"
        );
        self.add_comment(issue_id, &body).await
    }

    pub async fn create_issue(&self, title: &str, description: &str, team_id: Option<&str>) -> anyhow::Result<Value> {
        let tid = match team_id {
            Some(id) => id.to_string(),
            None => {
                let resp = self.graphql("query { teams { nodes { id name } } }", None).await?;
                resp["data"]["teams"]["nodes"][0]["id"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("no teams found"))?
                    .to_string()
            }
        };

        let query = r#"
            mutation($input: IssueCreateInput!) {
                issueCreate(input: $input) {
                    success
                    issue {
                        id
                        title
                        url
                    }
                }
            }
        "#;

        let input = json!({
            "title": title,
            "description": description,
            "teamId": tid,
        });

        let vars = json!({ "input": input });
        let resp = self.graphql(query, Some(vars)).await?;
        Ok(resp)
    }
}

use std::any::Any;
use std::sync::atomic::{AtomicU8, Ordering};
use std::pin::Pin;
use std::future::Future;
use tokio::sync::Mutex;
use crate::connector::{Connector, ConnectorStatus};

pub struct LinearConnector {
    config: Mutex<crate::config::LinearConfig>,
    client: Mutex<Option<LinearClient>>,
    status: AtomicU8,
    error_msg: Mutex<Option<String>>,
}

impl LinearConnector {
    pub fn new(config: crate::config::LinearConfig) -> Self {
        Self {
            config: Mutex::new(config),
            client: Mutex::new(None),
            status: AtomicU8::new(ConnectorStatus::Stopped.as_u8()),
            error_msg: Mutex::new(None),
        }
    }

    pub async fn client(&self) -> anyhow::Result<LinearClient> {
        self.client
            .lock()
            .await
            .clone()
            .ok_or_else(|| anyhow::anyhow!("linear connector not running"))
    }
}

impl Connector for LinearConnector {
    fn name(&self) -> &'static str {
        "linear"
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
            let new_client = LinearClient::new(&config.api_key);
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
                anyhow::bail!("linear connector not running")
            }
        })
    }

    fn reconfigure(
        &self,
        raw_toml: &toml::Value,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<bool>> + Send + '_>> {
        let result = raw_toml.clone().try_into::<crate::config::LinearConfig>();
        Box::pin(async move {
            let new_config = result.map_err(|e| anyhow::anyhow!("invalid linear config: {e}"))?;
            *self.config.lock().await = new_config;
            Ok(true)
        })
    }
}
