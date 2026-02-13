use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};

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

    async fn graphql(&self, query: &str, variables: Option<Value>) -> anyhow::Result<Value> {
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
