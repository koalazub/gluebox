use std::sync::Arc;
use crate::AppState;
use super::linear_from_registry;

pub async fn anytype_spec_changed(
    state: &Arc<AppState>,
    anytype_object_id: &str,
) -> anyhow::Result<()> {
    let Some(mapping) = state.db.get_spec_by_anytype_id(anytype_object_id).await? else {
        tracing::debug!(anytype_object_id, "no spec mapping for this anytype object");
        return Ok(());
    };

    let linear = linear_from_registry(state).await?;

    let comment = format!(
        "Spec updated in Anytype (object: `{}`). [View in Anytype](anytype://object/{})",
        anytype_object_id, anytype_object_id,
    );
    linear.add_comment(&mapping.linear_issue_id, &comment).await?;

    tracing::info!(
        anytype_object_id,
        linear_issue_id = %mapping.linear_issue_id,
        "trigger 4: anytype spec change notified to linear"
    );
    Ok(())
}
