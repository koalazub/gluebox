use crate::AppState;
use crate::connectors::matrix::MatrixBot;
use crate::connectors::opencode::{OpenCodeClient, IntentKind};
use crate::connectors::linear::LinearClient;
use std::sync::Arc;
use matrix_sdk::{
    config::SyncSettings,
    ruma::events::room::message::{
        MessageType,
        OriginalSyncRoomMessageEvent,
    },
    Room,
};

pub async fn start_openclaw(state: Arc<AppState>, bot: Arc<MatrixBot>) {
    let ai = match &state.cfg.opencode {
        Some(cfg) => Arc::new(OpenCodeClient::new(&cfg.api_key)),
        None => {
            tracing::warn!("opencode not configured, openclaw disabled");
            return;
        }
    };

    let bot_user_id = bot.client().user_id().map(|id| id.to_string()).unwrap_or_default();
    let target_room = bot.room_id().clone();

    let handler_state = state.clone();
    let handler_ai = ai.clone();
    let handler_bot = bot.clone();
    let handler_bot_id = bot_user_id.clone();

    bot.client().add_event_handler(move |event: OriginalSyncRoomMessageEvent, room: Room| {
        let state = handler_state.clone();
        let ai = handler_ai.clone();
        let bot = handler_bot.clone();
        let bot_id = handler_bot_id.clone();
        let target = target_room.clone();

        async move {
            if room.room_id() != target {
                return;
            }

            if event.sender.as_str() == bot_id {
                return;
            }

            let body = match &event.content.msgtype {
                MessageType::Text(text) => text.body.clone(),
                _ => return,
            };

            if !body.starts_with("!bot") {
                return;
            }

            let cleaned = body
                .strip_prefix("!bot")
                .unwrap_or(&body)
                .trim()
                .to_string();

            if cleaned.is_empty() {
                let _ = bot.send_message("What do you need? Just talk to me - I can draft specs, write decision records, file issues, or answer questions.").await;
                return;
            }

            tracing::info!(sender = %event.sender, body = %cleaned, "openclaw: processing message");

            if let Err(e) = handle_message(&state, &bot, &ai, &cleaned).await {
                tracing::error!(error = %e, "openclaw: failed to handle message");
                let _ = bot.send_message(&format!("Something went wrong: {e}")).await;
            }
        }
    });

    tracing::info!("openclaw: listening for @mentions and replies in encrypted room");

    bot.sync_forever(SyncSettings::default()).await;
}

fn fast_classify(msg: &str) -> Option<(IntentKind, String)> {
    let lower = msg.to_lowercase();
    let spec_signals = ["spec for", "spec about", "draft a spec", "write a spec", "design doc for", "design doc about"];
    let decision_signals = ["decision about", "decision on", "decide between", "decide whether", "decide if", "adr for", "adr about", "should we use"];
    let issue_signals = ["file an issue", "file a ticket", "create an issue", "create a ticket", "open an issue", "open a ticket", "track this", "add a task"];

    for signal in &spec_signals {
        if lower.contains(signal) {
            let prompt = strip_signal(&lower, signal, msg);
            return Some((IntentKind::Spec, prompt));
        }
    }
    for signal in &decision_signals {
        if lower.contains(signal) {
            let prompt = strip_signal(&lower, signal, msg);
            return Some((IntentKind::Decision, prompt));
        }
    }
    for signal in &issue_signals {
        if lower.contains(signal) {
            let prompt = strip_signal(&lower, signal, msg);
            return Some((IntentKind::Issue, prompt));
        }
    }
    None
}

fn strip_signal(lower: &str, signal: &str, original: &str) -> String {
    if let Some(pos) = lower.find(signal) {
        let after = pos + signal.len();
        let prompt = original[after..].trim().to_string();
        if prompt.is_empty() {
            original.to_string()
        } else {
            prompt
        }
    } else {
        original.to_string()
    }
}

async fn handle_message(
    state: &Arc<AppState>,
    bot: &Arc<MatrixBot>,
    ai: &Arc<OpenCodeClient>,
    message: &str,
) -> anyhow::Result<()> {
    let (kind, prompt) = match fast_classify(message) {
        Some((k, p)) => {
            tracing::info!(intent = ?k, "openclaw: fast-classified");
            (k, p)
        }
        None => {
            let intent = ai.classify_intent(message).await?;
            tracing::info!(intent = ?intent.kind, "openclaw: ai-classified");
            (intent.kind, intent.prompt)
        }
    };

    match kind {
        IntentKind::Spec => {
            bot.send_message("Drafting spec...").await?;
            let draft = ai.draft_spec(&prompt).await?;
            bot.send_markdown(&format!("**Draft Spec**\n\n{draft}")).await?;
        }
        IntentKind::Decision => {
            bot.send_message("Drafting decision record...").await?;
            let draft = ai.draft_decision(&prompt).await?;
            bot.send_markdown(&format!("**Draft Decision Record**\n\n{draft}")).await?;
        }
        IntentKind::Issue => {
            bot.send_message("Creating issue...").await?;
            let (title, description) = ai.draft_issue(&prompt).await?;
            let linear = LinearClient::new(&state.cfg.linear.api_key);
            let resp = linear.create_issue(&title, &description, None).await?;
            let url = resp["data"]["issueCreate"]["issue"]["url"]
                .as_str()
                .unwrap_or("(no url)");
            bot.send_markdown(&format!("**Created:** {title}\n{url}")).await?;
        }
        IntentKind::Chat => {
            let reply = ai.chat_reply(&prompt).await?;
            bot.send_markdown(&reply).await?;
        }
    }

    Ok(())
}
