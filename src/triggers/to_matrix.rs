use std::sync::Arc;
use crate::AppState;
use crate::connectors::matrix::{MatrixBot, MatrixConnector};

async fn get_bot(state: &Arc<AppState>) -> Option<Arc<MatrixBot>> {
    let conn = state.registry.get_dyn("matrix").await?;
    let matrix_connector = conn.as_any().downcast_ref::<MatrixConnector>()?;
    matrix_connector.bot().await.ok()
}

pub async fn notify_matrix(state: &Arc<AppState>, msg: &str) {
    let Some(bot) = get_bot(state).await else { return };
    if let Err(e) = bot.send_message(msg).await {
        tracing::error!(%e, "failed to send matrix notification");
    }
}

pub async fn notify_feedback_room(state: &Arc<AppState>, markdown: &str) {
    let Some(bot) = get_bot(state).await else { return };
    let feedback_room_id = {
        let cfg = state.config.read().await;
        let Some(ref matrix_cfg) = cfg.matrix else { return };
        matrix_cfg.feedback_room_id.clone()
    };
    let Some(ref room_id) = feedback_room_id else { return };
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
    let Some(bot) = get_bot(state).await else { return };
    let issues_room_id = {
        let cfg = state.config.read().await;
        let Some(ref matrix_cfg) = cfg.matrix else { return };
        matrix_cfg.issues_room_id.clone()
    };
    let Some(ref room_id) = issues_room_id else { return };
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
    if let Some(bot) = get_bot(state).await {
        bot.send_message(&msg).await?;
    }
    Ok(())
}
