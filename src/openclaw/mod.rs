use crate::AppState;
use crate::connectors::matrix::MatrixBot;
use crate::connectors::opencode::{OpenCodeClient, ExistingIssueSummary, FeedbackCluster, IntentKind};
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

    // !feedback always routes directly to the feedback pipeline
    if lower.starts_with("!feedback") || lower.starts_with("feedback:") {
        let body = msg
            .trim_start_matches("!feedback")
            .trim_start_matches("feedback:")
            .trim()
            .to_string();
        let body = if body.is_empty() { msg.to_string() } else { body };
        return Some((IntentKind::Feedback, body));
    }

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
        IntentKind::Feedback => {
            handle_feedback(state, bot, ai, &prompt).await?;
        }
        IntentKind::Chat => {
            let reply = ai.chat_reply(&prompt).await?;
            bot.send_markdown(&reply).await?;
        }
    }

    Ok(())
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

pub async fn handle_feedback(
    state: &Arc<AppState>,
    bot: &Arc<MatrixBot>,
    ai: &Arc<OpenCodeClient>,
    message: &str,
) -> anyhow::Result<()> {
    bot.send_message("Analysing feedback…").await?;

    let clusters = ai.extract_and_cluster_feedback(message).await?;

    if clusters.is_empty() {
        bot.send_message("No actionable feedback found in that message.").await?;
        return Ok(());
    }

    let results = process_feedback_clusters(state, ai, &clusters).await;

    let summary = format!(
        "Processed {} feedback cluster{}:\n\n{}",
        clusters.len(),
        if clusters.len() == 1 { "" } else { "s" },
        results.join("\n")
    );
    bot.send_markdown(&summary).await?;

    Ok(())
}

pub async fn process_feedback_clusters(
    state: &Arc<AppState>,
    ai: &Arc<OpenCodeClient>,
    clusters: &[FeedbackCluster],
) -> Vec<String> {
    let linear = LinearClient::new(&state.cfg.linear.api_key);

    let team_id = match state.cfg.linear.team_id.as_deref() {
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

        let description = build_issue_description(cluster);

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

    results
}

fn build_issue_description(cluster: &FeedbackCluster) -> String {
    let items = cluster.items
        .iter()
        .map(|i| format!("- {i}"))
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "{}\n\n**Reported feedback items:**\n{}",
        cluster.description, items
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connectors::opencode::IntentKind;

    #[test]
    fn feedback_bang_prefix() {
        let (kind, body) = fast_classify("!feedback login crashes on iOS").unwrap();
        assert!(matches!(kind, IntentKind::Feedback));
        assert_eq!(body, "login crashes on iOS");
    }

    #[test]
    fn feedback_colon_prefix() {
        let (kind, body) = fast_classify("feedback: dark mode is missing").unwrap();
        assert!(matches!(kind, IntentKind::Feedback));
        assert_eq!(body, "dark mode is missing");
    }

    #[test]
    fn feedback_bang_no_body_returns_original() {
        let (kind, body) = fast_classify("!feedback").unwrap();
        assert!(matches!(kind, IntentKind::Feedback));
        assert_eq!(body, "!feedback");
    }

    #[test]
    fn spec_signal_detected() {
        let (kind, _) = fast_classify("Can you write a spec for the new auth flow?").unwrap();
        assert!(matches!(kind, IntentKind::Spec));
    }

    #[test]
    fn spec_draft_signal_detected() {
        let (kind, prompt) = fast_classify("draft a spec for pagination").unwrap();
        assert!(matches!(kind, IntentKind::Spec));
        assert_eq!(prompt, "pagination");
    }

    #[test]
    fn decision_signal_detected() {
        let (kind, _) = fast_classify("should we use postgres or sqlite?").unwrap();
        assert!(matches!(kind, IntentKind::Decision));
    }

    #[test]
    fn adr_signal_detected() {
        let (kind, prompt) = fast_classify("adr for using tokio over async-std").unwrap();
        assert!(matches!(kind, IntentKind::Decision));
        assert_eq!(prompt, "using tokio over async-std");
    }

    #[test]
    fn issue_signal_detected() {
        let (kind, _) = fast_classify("file an issue for the broken login page").unwrap();
        assert!(matches!(kind, IntentKind::Issue));
    }

    #[test]
    fn track_this_signal_detected() {
        let (kind, prompt) = fast_classify("track this: memory leak in worker").unwrap();
        assert!(matches!(kind, IntentKind::Issue));
        assert_eq!(prompt.trim_start_matches(':').trim(), "memory leak in worker");
    }

    #[test]
    fn no_signal_returns_none() {
        assert!(fast_classify("hello there, how are you?").is_none());
        assert!(fast_classify("what time is it?").is_none());
    }

    #[test]
    fn strip_signal_extracts_suffix() {
        let result = strip_signal("spec for the auth flow", "spec for", "spec for the auth flow");
        assert_eq!(result, "the auth flow");
    }

    #[test]
    fn strip_signal_empty_suffix_returns_original() {
        let result = strip_signal("spec for", "spec for", "spec for");
        assert_eq!(result, "spec for");
    }
}
