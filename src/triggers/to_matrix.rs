use std::sync::Arc;
use crate::AppState;

#[allow(dead_code)]
pub async fn notify_state_change(
    state: &Arc<AppState>,
    issue_title: &str,
    new_state: &str,
    linear_url: &str,
) -> anyhow::Result<()> {
    let msg = format!("[{new_state}] {issue_title}\n{linear_url}");

    if let Some(bot) = &state.matrix_bot {
        bot.send_message(&msg).await?;
    }

    tracing::info!(issue_title, new_state, "trigger 5: state change pinged to matrix");
    Ok(())
}

pub async fn notify_contract_event(
    state: &Arc<AppState>,
    event: &str,
    title: &str,
    detail: &str,
) -> anyhow::Result<()> {
    let msg = format!("[Documenso: {event}] {title}\n{detail}");

    if let Some(bot) = &state.matrix_bot {
        bot.send_message(&msg).await?;
    }

    Ok(())
}
