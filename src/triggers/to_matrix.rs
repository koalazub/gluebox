use std::sync::Arc;
use crate::AppState;

pub async fn notify_matrix(state: &Arc<AppState>, msg: &str) {
    let Some(bot) = &state.matrix_bot else { return };
    if let Err(e) = bot.send_message(msg).await {
        tracing::error!(%e, "failed to send matrix notification");
    }
}

pub async fn notify_feedback_room(state: &Arc<AppState>, markdown: &str) {
    let (Some(bot), Some(room_id)) = (&state.matrix_bot, &state.cfg.matrix.feedback_room_id) else {
        return;
    };
    if let Err(e) = bot.send_markdown_to_room(room_id, markdown).await {
        tracing::error!(%e, "failed to send to matrix feedback room");
    }
}

pub async fn notify_ticket_created(
    state: &Arc<AppState>,
    title: &str,
    primary_url: &str,
    cross_link: Option<(&str, &str)>,
) {
    let (Some(bot), Some(room_id)) = (&state.matrix_bot, &state.cfg.matrix.issues_room_id) else {
        return;
    };
    let msg = match cross_link {
        Some((label, url)) => format!("New ticket: {title}\n{primary_url}\n{label}: {url}"),
        None => format!("New ticket: {title}\n{primary_url}"),
    };
    if let Err(e) = bot.send_to_room(room_id, &msg).await {
        tracing::error!(%e, "failed to send to matrix issues room");
    }
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
