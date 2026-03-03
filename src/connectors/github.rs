use reqwest::Client;
use serde::Deserialize;
use serde_json::{Value, json};

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
