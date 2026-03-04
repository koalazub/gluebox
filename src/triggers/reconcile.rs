use std::sync::Arc;
use crate::AppState;
use crate::connectors::anytype::AnytypeClient;
use crate::connectors::linear::LinearClient;

pub async fn run_nightly(state: &Arc<AppState>) -> anyhow::Result<()> {
    tracing::info!("trigger 8: starting nightly reconciliation");

    let missing_anytype = state.db.specs_missing_anytype_link().await?;
    let missing_linear = state.db.specs_missing_linear_id().await?;

    tracing::info!(
        missing_anytype = missing_anytype.len(),
        missing_linear = missing_linear.len(),
        "reconciliation scan complete"
    );

    if let Some(ref at_cfg) = state.cfg.anytype {
        let anytype = AnytypeClient::new(&at_cfg.api_url, &at_cfg.api_key, &at_cfg.space_id);
        let linear = LinearClient::new(&state.cfg.linear.api_key);

        for spec in &missing_anytype {
            tracing::info!(linear_id = %spec.linear_issue_id, "reconcile: spec missing anytype link");
            let issue = linear.get_issue(&spec.linear_issue_id).await?;
            let body_md = format!(
                "**Linear Issue:** [{title}]({url})\n\n{desc}",
                title = issue.title,
                url = issue.url,
                desc = issue.description.as_deref().unwrap_or(""),
            );
            let obj = anytype.create_object(
                "spec", &issue.title,
                issue.description.as_deref().unwrap_or(""),
                Some(&body_md),
            ).await?;

            state.db.upsert_spec(&crate::db::SpecMapping {
                linear_issue_id: spec.linear_issue_id.clone(),
                anytype_object_id: Some(obj.id.clone()),
                linear_url: Some(issue.url),
                anytype_url: None,
                last_synced_at: None,
            }).await?;

            tracing::info!(linear_id = %spec.linear_issue_id, anytype_id = %obj.id, "reconcile: created missing anytype spec");
        }
    } else if !missing_anytype.is_empty() {
        tracing::info!(count = missing_anytype.len(), "reconcile: anytype not configured, skipping spec creation");
    }

    for spec in &missing_linear {
        if let Some(ref anytype_id) = spec.anytype_object_id {
            tracing::warn!(anytype_id, "reconcile: anytype spec missing linear id - manual fix needed");
        }
    }

    tracing::info!("nightly reconciliation complete");
    Ok(())
}
