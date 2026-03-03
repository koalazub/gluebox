use std::sync::Arc;
use serde_json::{json, Value};
use crate::AppState;
use crate::connectors::anytype::AnytypeClient;
use crate::connectors::linear::LinearClient;
use crate::db::SpecMapping;
use super::to_matrix;

fn has_label(payload: &Value, label_name: &str) -> bool {
    payload["data"]["labels"]
        .as_array()
        .map(|labels| labels.iter().any(|l| {
            l["name"].as_str().map(|n| n.eq_ignore_ascii_case(label_name)).unwrap_or(false)
        }))
        .unwrap_or(false)
}

fn state_name(payload: &Value) -> Option<&str> {
    payload["data"]["state"]["name"].as_str()
}

fn state_type(payload: &Value) -> Option<&str> {
    payload["data"]["state"]["type"].as_str()
}

pub async fn linear_issue_created(state: &Arc<AppState>, payload: &Value) -> anyhow::Result<()> {
    if !has_label(payload, "spec") {
        tracing::debug!("issue created without 'spec' label, skipping");
        return Ok(());
    }

    let issue_id = payload["data"]["id"].as_str().unwrap_or_default();
    let title = payload["data"]["title"].as_str().unwrap_or_default();
    let description = payload["data"]["description"].as_str().unwrap_or_default();
    let url = payload["url"].as_str().unwrap_or_default();

    tracing::info!(issue_id, title, "trigger 1: spec-labeled issue created, upserting anytype Spec");

    let anytype = AnytypeClient::new(
        &state.cfg.anytype.api_url,
        &state.cfg.anytype.api_key,
        &state.cfg.anytype.space_id,
    );

    let body_md = format!("**Linear Issue:** [{title}]({url})\n\n{description}");
    let obj = match anytype.create_object("spec", title, description, Some(&body_md)).await {
        Ok(o) => o,
        Err(e) => {
            tracing::error!(%e, "failed to create anytype spec object");
            return Err(e);
        }
    };

    state.db.upsert_spec(&SpecMapping {
        linear_issue_id: issue_id.to_string(),
        anytype_object_id: Some(obj.id.clone()),
        linear_url: Some(url.to_string()),
        anytype_url: None,
        last_synced_at: None,
    }).await?;

    let linear = LinearClient::new(&state.cfg.linear.api_key);
    let link_text = format!(
        "{description}\n\n---\n**Anytype Spec:** `{}`",
        obj.id,
    );
    linear.update_issue_description(issue_id, &link_text).await?;

    tracing::info!(issue_id, anytype_id = %obj.id, "spec created in anytype, link written back to linear");
    Ok(())
}

pub async fn linear_issue_updated(state: &Arc<AppState>, payload: &Value) -> anyhow::Result<()> {
    let issue_id = payload["data"]["id"].as_str().unwrap_or_default();

    let Some(mapping) = state.db.get_spec_by_linear_id(issue_id).await? else {
        tracing::debug!(issue_id, "no spec mapping for this issue, skipping");
        return Ok(());
    };

    let Some(anytype_id) = &mapping.anytype_object_id else {
        tracing::debug!(issue_id, "spec mapping has no anytype id yet, skipping");
        return Ok(());
    };

    let title = payload["data"]["title"].as_str();
    let description = payload["data"]["description"].as_str();
    let _priority = payload["data"]["priority"].as_f64();
    let current_state = state_name(payload);

    let mut updates = json!({});
    if let Some(t) = title {
        updates["name"] = json!(t);
    }
    if let Some(d) = description {
        updates["description"] = json!(d);
    }

    let anytype = AnytypeClient::new(
        &state.cfg.anytype.api_url,
        &state.cfg.anytype.api_key,
        &state.cfg.anytype.space_id,
    );

    if updates.as_object().map(|o| !o.is_empty()).unwrap_or(false) {
        tracing::info!(issue_id, anytype_id, "trigger 2: patching anytype spec");
        anytype.update_object(anytype_id, updates).await?;
    }

    if let Some(stype) = state_type(payload) {
        if stype == "completed" || current_state == Some("Shipped") || current_state == Some("Done") {
            tracing::info!(issue_id, "trigger 3: issue shipped, updating anytype + matrix");

            anytype.update_object(anytype_id, json!({
                "description": format!("Status: Shipped\n\n{}",
                    description.unwrap_or_default()),
            })).await?;

            let issue_title = title.unwrap_or("(untitled)");
            let linear_url = payload["url"].as_str().unwrap_or("");

            to_matrix::notify_matrix(state, &format!("Shipped: {issue_title}\n{linear_url}")).await;

            tracing::info!(issue_id, "shipped notification sent to matrix");
        }

        if current_state == Some("In Progress") || current_state == Some("In Review") {
            let issue_title = title.unwrap_or("(untitled)");
            let linear_url = payload["url"].as_str().unwrap_or("");
            to_matrix::notify_matrix(state, &format!("{}: {issue_title}\n{linear_url}", current_state.unwrap_or("Updated"))).await;
        }
    }

    Ok(())
}
