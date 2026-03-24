use std::sync::Arc;
use serde_json::Value;
use crate::AppState;
use super::to_matrix;
use super::linear_from_registry;

pub async fn github_issue_opened(state: &Arc<AppState>, payload: &Value) -> anyhow::Result<()> {
    let number = payload["issue"]["number"].as_i64().unwrap_or(0);
    let title = payload["issue"]["title"].as_str().unwrap_or_default();
    let body = payload["issue"]["body"].as_str().unwrap_or_default();
    let html_url = payload["issue"]["html_url"].as_str().unwrap_or_default();
    let repo = payload["repository"]["full_name"].as_str().unwrap_or_default();

    if state.db.get_linear_issue_for_github(number, repo).await?.is_some() {
        tracing::debug!(number, repo, "github issue already mapped to linear, skipping");
        return Ok(());
    }

    let linear = linear_from_registry(state).await?;
    let team_id = {
        let cfg = state.config.read().await;
        cfg.linear.as_ref().and_then(|c| c.team_id.clone())
    };
    let description = format!("Synced from GitHub: {html_url}\n\n{body}");
    let resp = linear.create_issue(title, &description, team_id.as_deref()).await?;

    let linear_id = resp["data"]["issueCreate"]["issue"]["id"].as_str().unwrap_or_default();
    let linear_url = resp["data"]["issueCreate"]["issue"]["url"].as_str().unwrap_or_default();

    state.db.insert_github_linear_mapping(number, repo, linear_id, Some(linear_url)).await?;

    to_matrix::notify_ticket_created(state, title, html_url, Some(("Linear", linear_url))).await;

    tracing::info!(number, repo, linear_id, "github issue synced to linear");
    Ok(())
}
