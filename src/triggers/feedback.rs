use std::sync::Arc;

use crate::AppState;
use crate::connectors::opencode::{OpenCodeClient, ExistingIssueSummary, FeedbackCluster};
use crate::triggers::{linear_from_registry, to_matrix};

pub struct FeedbackContext {
    pub user: String,
    pub user_id: String,
    pub username: String,
    pub submitted_at: String,
    pub url: String,
    pub user_agent: String,
    pub frontend_logs: String,
    pub backend_logs: String,
    pub screenshot_id: String,
}

fn category_color(category: &str) -> &'static str {
    match category {
        "bug"         => "#e11d48",
        "feature"     => "#7c3aed",
        "ux"          => "#0891b2",
        "performance" => "#d97706",
        "docs"        => "#16a34a",
        _             => "#6b7280",
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
    }
}

pub async fn process_feedback_clusters(
    state: &Arc<AppState>,
    ai: &Arc<OpenCodeClient>,
    clusters: &[FeedbackCluster],
    context: Option<&FeedbackContext>,
) -> Vec<String> {
    let linear = match linear_from_registry(state).await {
        Ok(c) => c,
        Err(_) => {
            tracing::error!("feedback: linear connector not available");
            return vec!["Linear not available — cannot process feedback".to_string()];
        }
    };

    let linear_team_id_opt = {
        let cfg = state.config.read().await;
        cfg.linear.as_ref().and_then(|c| c.team_id.clone())
    };

    let team_id = match linear_team_id_opt.as_deref() {
        Some(id) => id.to_string(),
        None => {
            match linear.graphql("query { teams { nodes { id } } }", None).await {
                Ok(resp) => resp["data"]["teams"]["nodes"][0]["id"]
                    .as_str()
                    .unwrap_or("")
                    .to_string(),
                Err(e) => {
                    tracing::error!(error = %e, "feedback: failed to resolve linear team_id");
                    return vec![format!("Could not resolve Linear team: {e}")];
                }
            }
        }
    };

    let mut results: Vec<String> = Vec::new();

    for cluster in clusters {
        let label_name = capitalize(&cluster.category);
        let color = category_color(&cluster.category);

        let existing_db = match state.db.get_feedback_by_category(&cluster.category, 20).await {
            Ok(v) => v,
            Err(e) => {
                tracing::error!(error = %e, "feedback: db query failed");
                results.push(format!("**[{}]** DB error: {e}", label_name));
                continue;
            }
        };

        let existing_summaries: Vec<ExistingIssueSummary> = existing_db
            .iter()
            .map(|t| ExistingIssueSummary {
                id: t.linear_issue_id.clone(),
                title: t.title.clone(),
                category: t.category.clone(),
            })
            .collect();

        let duplicate_id = if existing_summaries.is_empty() {
            None
        } else {
            ai.find_duplicate_issue(cluster, &existing_summaries).await.unwrap_or(None)
        };

        if let Some(ref dup_id) = duplicate_id {
            let existing = existing_db.iter().find(|t| &t.linear_issue_id == dup_id);
            let url = existing.map(|t| t.linear_issue_url.as_str()).unwrap_or("(unknown)");

            match linear.add_feedback_comment(dup_id, &cluster.items, "via OpenClaw").await {
                Ok(()) => {
                    tracing::info!(issue_id = %dup_id, "feedback: added comment to existing issue");
                    results.push(format!(
                        "**[{}] Duplicate** — commented on existing issue: {}",
                        label_name, url
                    ));
                }
                Err(e) => {
                    tracing::error!(error = %e, "feedback: failed to comment on existing issue");
                    results.push(format!(
                        "**[{}]** Could not comment on existing issue {}: {e}",
                        label_name, url
                    ));
                }
            }
            continue;
        }

        let label_id = match linear.get_or_create_label(&team_id, &label_name, color).await {
            Ok(id) => id,
            Err(e) => {
                tracing::error!(error = %e, category = %cluster.category, "feedback: failed to get/create label");
                results.push(format!("**[{}]** Could not create label: {e}", label_name));
                continue;
            }
        };

        let description = build_issue_description(cluster, context);

        match linear.create_issue_with_label(&cluster.title, &description, &team_id, &label_id).await {
            Ok(resp) => {
                let issue_id = resp["data"]["issueCreate"]["issue"]["id"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();
                let url = resp["data"]["issueCreate"]["issue"]["url"]
                    .as_str()
                    .unwrap_or("(no url)")
                    .to_string();

                if !issue_id.is_empty() {
                    let _ = state.db.insert_feedback_ticket(
                        &issue_id,
                        &url,
                        &cluster.title,
                        &cluster.category,
                        &cluster.description,
                    ).await;
                }

                tracing::info!(issue_id = %issue_id, title = %cluster.title, "feedback: created linear issue");
                results.push(format!("**[{}]** {} — {}", label_name, cluster.title, url));
            }
            Err(e) => {
                tracing::error!(error = %e, title = %cluster.title, "feedback: failed to create issue");
                results.push(format!("**[{}]** Failed to create issue for \"{}\": {e}", label_name, cluster.title));
            }
        }
    }

    if !results.is_empty() {
        let summary = format!(
            "**Feedback processed** ({} cluster{}):\n\n{}",
            clusters.len(),
            if clusters.len() == 1 { "" } else { "s" },
            results.join("\n"),
        );
        to_matrix::notify_feedback_room(state, &summary).await;
    }

    results
}

fn build_issue_description(
    cluster: &FeedbackCluster,
    context: Option<&FeedbackContext>,
) -> String {
    let items = cluster.items
        .iter()
        .map(|i| format!("- {i}"))
        .collect::<Vec<_>>()
        .join("\n");

    let mut desc = format!(
        "{}\n\n**Reported feedback items:**\n{}",
        cluster.description, items
    );

    if let Some(ctx) = context {
        if !ctx.url.is_empty() {
            desc.push_str(&format!("\n\n**Page:** {}", ctx.url));
        }

        let mut reporter_parts = vec![];
        if !ctx.username.is_empty() && ctx.username != "anonymous" {
            reporter_parts.push(format!("@{}", ctx.username));
        } else if !ctx.user_id.is_empty() && ctx.user_id != "anonymous" {
            reporter_parts.push(format!("ID: {}", ctx.user_id));
        } else if !ctx.user.is_empty() {
            reporter_parts.push(ctx.user.clone());
        }

        if !ctx.submitted_at.is_empty() {
            reporter_parts.push(format!("submitted at {}", ctx.submitted_at));
        }

        if !reporter_parts.is_empty() {
            desc.push_str(&format!("\n**Reporter:** {}", reporter_parts.join(" ")));
        }

        if !ctx.user_agent.is_empty() {
            desc.push_str(&format!("\n**User Agent:** {}", ctx.user_agent));
        }

        let fe_errors: Vec<&str> = ctx.frontend_logs
            .lines()
            .filter(|l| l.contains("ERROR") || l.contains("WARN"))
            .collect();
        if !fe_errors.is_empty() {
            desc.push_str("\n\n**Frontend errors (last 5 min):**\n```\n");
            for line in &fe_errors[..fe_errors.len().min(30)] {
                desc.push_str(line);
                desc.push('\n');
            }
            desc.push_str("```");
        }

        let be_errors: Vec<&str> = ctx.backend_logs
            .lines()
            .filter(|l| l.contains("ERROR") || l.contains("WARN"))
            .collect();
        if !be_errors.is_empty() {
            desc.push_str("\n\n**Backend errors (last 5 min):**\n```\n");
            for line in &be_errors[..be_errors.len().min(30)] {
                desc.push_str(line);
                desc.push('\n');
            }
            desc.push_str("```");
        }
        if !ctx.screenshot_id.is_empty() {
            desc.push_str(&format!("\n\n**Screenshot:** ID `{}` (retrieve from blob storage)", ctx.screenshot_id));
        }
    }

    desc
}
