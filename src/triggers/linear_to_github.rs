use std::sync::Arc;
use serde_json::Value;
use crate::AppState;
use super::to_matrix;
use super::github_from_registry;

pub async fn linear_issue_github_sync(state: &Arc<AppState>, payload: &Value) -> anyhow::Result<()> {
    let issue_id = payload["data"]["id"].as_str().unwrap_or_default();
    let title = payload["data"]["title"].as_str().unwrap_or_default();
    let description = payload["data"]["description"].as_str().unwrap_or_default();
    let linear_url = payload["url"].as_str().unwrap_or_default();

    if state.db.get_github_issue_for_linear(issue_id).await?.is_some() {
        tracing::debug!(issue_id, "linear issue already mapped to github, skipping");
        return Ok(());
    }

    let gh = github_from_registry(state).await?;
    let body = format!("Synced from Linear: {linear_url}\n\n{description}");
    let issue = gh.create_issue(title, &body, &["linear-sync"]).await?;

    let gh_repo = {
        let cfg = state.config.read().await;
        cfg.github.as_ref()
            .map(|c| c.repo.clone())
            .unwrap_or_default()
    };

    state.db.insert_github_linear_mapping(issue.number, &gh_repo, issue_id, Some(linear_url)).await?;

    to_matrix::notify_ticket_created(state, title, linear_url, Some(("GitHub", &issue.html_url))).await;

    tracing::info!(issue_id, gh_number = issue.number, "linear issue synced to github");
    Ok(())
}
