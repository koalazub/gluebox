use std::sync::Arc;
use serde_json::json;
use crate::AppState;
use crate::connectors::anytype::AnytypeClient;
use crate::connectors::documenso::WebhookPayload;
use crate::db::ContractMapping;
use super::to_matrix;

pub async fn documenso_completed(state: &Arc<AppState>, payload: &WebhookPayload) -> anyhow::Result<()> {
    let doc = &payload.payload;
    let doc_id = doc.id.to_string();

    tracing::info!(doc_id, title = %doc.title, "trigger 6: document completed");

    let parties = doc.recipients.as_ref()
        .map(|r| r.iter().map(|p| format!("{} <{}>", p.name, p.email)).collect::<Vec<_>>().join(", "))
        .unwrap_or_default();

    let description = format!(
        "Status: Completed\nParties: {parties}\nCompleted: {}",
        doc.completed_at.as_deref().unwrap_or("unknown"),
    );

    let existing = state.db.get_contract_by_documenso_id(&doc_id).await?;

    let anytype_id = if let Some(ref at_cfg) = state.cfg.anytype {
        let anytype = AnytypeClient::new(&at_cfg.api_url, &at_cfg.api_key, &at_cfg.space_id);
        if let Some(ref m) = existing {
            if let Some(ref aid) = m.anytype_object_id {
                anytype.update_object(aid, json!({
                    "name": doc.title,
                    "description": description,
                })).await?;
                Some(aid.clone())
            } else {
                let obj = anytype.create_object("contract", &doc.title, &description, None).await?;
                Some(obj.id)
            }
        } else {
            let obj = anytype.create_object("contract", &doc.title, &description, None).await?;
            Some(obj.id)
        }
    } else {
        existing.as_ref().and_then(|m| m.anytype_object_id.clone())
    };

    state.db.upsert_contract(&ContractMapping {
        documenso_document_id: doc_id.clone(),
        anytype_object_id: anytype_id,
        linear_issue_id: existing.and_then(|m| m.linear_issue_id),
        status: Some("completed".to_string()),
        last_synced_at: None,
    }).await?;

    to_matrix::notify_contract_event(
        state, "Completed", &doc.title, &parties,
    ).await?;

    Ok(())
}

pub async fn documenso_rejected(state: &Arc<AppState>, payload: &WebhookPayload) -> anyhow::Result<()> {
    let doc = &payload.payload;
    let doc_id = doc.id.to_string();

    tracing::info!(doc_id, title = %doc.title, "trigger 7: document rejected/expired");

    let rejection_reasons: Vec<String> = doc.recipients.as_ref()
        .map(|r| r.iter()
            .filter(|p| p.signing_status == "REJECTED")
            .map(|p| format!("{}: {}", p.name, p.rejection_reason.as_deref().unwrap_or("no reason")))
            .collect())
        .unwrap_or_default();

    let description = format!(
        "Status: Rejected\nReasons: {}",
        if rejection_reasons.is_empty() { "unknown".to_string() } else { rejection_reasons.join("; ") },
    );

    let existing = state.db.get_contract_by_documenso_id(&doc_id).await?;

    if let Some(ref at_cfg) = state.cfg.anytype {
        if let Some(ref m) = existing {
            if let Some(ref aid) = m.anytype_object_id {
                let anytype = AnytypeClient::new(&at_cfg.api_url, &at_cfg.api_key, &at_cfg.space_id);
                anytype.update_object(aid, json!({
                    "description": description,
                })).await?;
            }
        }
    }

    state.db.upsert_contract(&ContractMapping {
        documenso_document_id: doc_id.clone(),
        anytype_object_id: existing.as_ref().and_then(|m| m.anytype_object_id.clone()),
        linear_issue_id: existing.as_ref().and_then(|m| m.linear_issue_id.clone()),
        status: Some("rejected".to_string()),
        last_synced_at: None,
    }).await?;

    to_matrix::notify_contract_event(
        state, "Rejected", &doc.title, &description,
    ).await?;

    if let Some(ref m) = existing {
        if let Some(ref linear_id) = m.linear_issue_id {
            let linear = crate::connectors::linear::LinearClient::new(&state.cfg.linear.api_key);
            linear.add_comment(
                linear_id,
                &format!("Contract rejected: {}\n{}", doc.title, description),
            ).await?;
        }
    }

    Ok(())
}
