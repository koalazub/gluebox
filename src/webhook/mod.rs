mod verify;

use std::sync::Arc;
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use bytes::Bytes;
use serde::Deserialize;

use crate::AppState;
use crate::triggers;
use crate::triggers::to_matrix;
use crate::connectors::opencode::OpenCodeClient;
use crate::openclaw::process_feedback_clusters;

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/webhooks/linear", post(handle_linear))
        .route("/webhooks/documenso", post(handle_documenso))
        .route("/webhooks/github", post(handle_github))
        .route("/api/notify", post(handle_notify))
        .route("/api/feedback", post(handle_feedback))
        .with_state(state)
}

async fn health() -> &'static str {
    "ok"
}

async fn handle_linear(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> StatusCode {
    let signature = headers
        .get("linear-signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !verify::linear_signature(signature, &body, &state.cfg.linear.webhook_secret) {
        tracing::warn!("linear webhook signature verification failed");
        return StatusCode::UNAUTHORIZED;
    }

    let payload: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(%e, "failed to parse linear webhook body");
            return StatusCode::BAD_REQUEST;
        }
    };

    let timestamp = payload["webhookTimestamp"].as_i64().unwrap_or(0);
    let now_ms = chrono::Utc::now().timestamp_millis();
    if (now_ms - timestamp).unsigned_abs() > 60_000 {
        tracing::warn!(timestamp, now_ms, "linear webhook timestamp too old");
        return StatusCode::UNAUTHORIZED;
    }

    let action = payload["action"].as_str().unwrap_or("");
    let event_type = payload["type"].as_str().unwrap_or("");

    tracing::info!(action, event_type, "linear webhook received");

    if let Err(e) = state.db.log_event("linear", &format!("{event_type}.{action}"),
        payload["data"]["id"].as_str().unwrap_or("unknown"),
        Some(&body.iter().map(|&b| b as char).collect::<String>())).await {
        tracing::error!(%e, "failed to log linear event");
    }

    let result = match (event_type, action) {
        ("Issue", "create") => {
            let title = payload["data"]["title"].as_str().unwrap_or("(untitled)");
            let url = payload["url"].as_str().unwrap_or("");
            to_matrix::notify_ticket_created(&state, title, url, None).await;

            let spec_result = triggers::linear_issue_created(&state, &payload).await;
            if let Err(e) = triggers::linear_issue_github_sync(&state, &payload).await {
                tracing::error!(%e, "linear→github sync failed");
            }
            spec_result
        }
        ("Issue", "update") => triggers::linear_issue_updated(&state, &payload).await,
        _ => {
            tracing::debug!(event_type, action, "unhandled linear event");
            Ok(())
        }
    };

    match result {
        Ok(()) => StatusCode::OK,
        Err(e) => {
            tracing::error!(%e, "trigger processing failed");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

async fn handle_github(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> StatusCode {
    let Some(gh_cfg) = &state.cfg.github else {
        return StatusCode::NOT_FOUND;
    };

    let signature = headers
        .get("x-hub-signature-256")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !verify::github_signature(signature, &body, &gh_cfg.webhook_secret) {
        tracing::warn!("github webhook signature verification failed");
        return StatusCode::UNAUTHORIZED;
    }

    let payload: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(%e, "failed to parse github webhook body");
            return StatusCode::BAD_REQUEST;
        }
    };

    let event = headers
        .get("x-github-event")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let action = payload["action"].as_str().unwrap_or("");

    tracing::info!(event, action, "github webhook received");

    if let Err(e) = state.db.log_event(
        "github",
        &format!("{event}.{action}"),
        &payload["issue"]["number"].to_string(),
        None,
    ).await {
        tracing::error!(%e, "failed to log github event");
    }

    let result = match (event, action) {
        ("issues", "opened") => triggers::github_issue_opened(&state, &payload).await,
        _ => {
            tracing::debug!(event, action, "unhandled github event");
            Ok(())
        }
    };

    match result {
        Ok(()) => StatusCode::OK,
        Err(e) => {
            tracing::error!(%e, "github trigger processing failed");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

async fn handle_documenso(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> StatusCode {
    let secret = headers
        .get("x-documenso-secret")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !verify::documenso_secret(secret, &state.cfg.documenso.webhook_secret) {
        tracing::warn!("documenso webhook secret verification failed");
        return StatusCode::UNAUTHORIZED;
    }

    let payload: crate::connectors::documenso::WebhookPayload = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(%e, "failed to parse documenso webhook body");
            return StatusCode::BAD_REQUEST;
        }
    };

    tracing::info!(event = %payload.event, doc_id = %payload.payload.id, "documenso webhook received");

    if let Err(e) = state.db.log_event("documenso", &payload.event,
        &payload.payload.id.to_string(), None).await {
        tracing::error!(%e, "failed to log documenso event");
    }

    let result = match payload.event.as_str() {
        "DOCUMENT_COMPLETED" => triggers::documenso_completed(&state, &payload).await,
        "DOCUMENT_REJECTED" => triggers::documenso_rejected(&state, &payload).await,
        _ => {
            tracing::debug!(event = %payload.event, "unhandled documenso event");
            Ok(())
        }
    };

    match result {
        Ok(()) => StatusCode::OK,
        Err(e) => {
            tracing::error!(%e, "documenso trigger processing failed");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

#[derive(Deserialize)]
struct NotifyRequest {
    room_id: Option<String>,
    message: String,
    #[serde(default)]
    format: NotifyFormat,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "lowercase")]
enum NotifyFormat {
    #[default]
    Plain,
    Markdown,
}

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

#[derive(Deserialize)]
struct FeedbackRequest {
    message: String,
    #[serde(default)]
    category: String,
    #[serde(default)]
    user: String,
    #[serde(default, rename = "user_id")]
    user_id: String,
    #[serde(default, rename = "username")]
    username: String,
    #[serde(default, rename = "submitted_at")]
    submitted_at: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    user_agent: String,
    #[serde(default)]
    frontend_logs: String,
    #[serde(default)]
    backend_logs: String,
    #[serde(default)]
    screenshot_id: String,
}

async fn handle_feedback(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<FeedbackRequest>,
) -> StatusCode {
    let Some(ref secret) = state.cfg.notify_secret else {
        return StatusCode::NOT_FOUND;
    };

    let provided = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");

    if !verify::constant_time_eq_pub(provided.as_bytes(), secret.as_bytes()) {
        return StatusCode::UNAUTHORIZED;
    }

    let Some(ref opencode_cfg) = state.cfg.opencode else {
        return StatusCode::SERVICE_UNAVAILABLE;
    };

    let api_key = opencode_cfg.api_key.clone();

    tokio::spawn(async move {
        let ai = Arc::new(OpenCodeClient::new(&api_key));

        let mut llm_input = req.message.clone();
        if !req.url.is_empty() {
            llm_input = format!("Page: {}\n\n{}", req.url, llm_input);
        }
        if !req.frontend_logs.is_empty() {
            let error_lines: Vec<&str> = req.frontend_logs
                .lines()
                .filter(|l| l.contains("ERROR") || l.contains("WARN"))
                .collect();
            if !error_lines.is_empty() {
                llm_input.push_str("\n\nFrontend errors:\n");
                llm_input.push_str(&error_lines.join("\n"));
            }
        }
        if !req.backend_logs.is_empty() {
            let error_lines: Vec<&str> = req.backend_logs
                .lines()
                .filter(|l| l.contains("ERROR") || l.contains("WARN"))
                .collect();
            if !error_lines.is_empty() {
                llm_input.push_str("\n\nBackend errors:\n");
                llm_input.push_str(&error_lines.join("\n"));
            }
        }

        match ai.extract_and_cluster_feedback(&llm_input).await {
            Ok(clusters) if !clusters.is_empty() => {
                let context = FeedbackContext {
                    user: req.user,
                    user_id: req.user_id,
                    username: req.username,
                    submitted_at: req.submitted_at,
                    url: req.url,
                    user_agent: req.user_agent,
                    frontend_logs: req.frontend_logs,
                    backend_logs: req.backend_logs,
                    screenshot_id: req.screenshot_id,
                };
                let results = process_feedback_clusters(&state, &ai, &clusters, Some(&context)).await;
                tracing::info!(clusters = clusters.len(), ?results, "feedback pipeline complete");
            }
            Ok(_) => tracing::info!("feedback pipeline: no clusters extracted"),
            Err(e) => tracing::error!(error = %e, "feedback pipeline: clustering failed"),
        }
    });

    StatusCode::ACCEPTED
}

async fn handle_notify(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<NotifyRequest>,
) -> StatusCode {
    let Some(ref secret) = state.cfg.notify_secret else {
        tracing::warn!("notify endpoint called but notify_secret not configured");
        return StatusCode::NOT_FOUND;
    };

    let provided = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");

    if !verify::constant_time_eq_pub(provided.as_bytes(), secret.as_bytes()) {
        tracing::warn!("notify endpoint: invalid bearer token");
        return StatusCode::UNAUTHORIZED;
    }

    let Some(bot) = &state.matrix_bot else {
        tracing::error!("notify endpoint: matrix bot not initialised");
        return StatusCode::SERVICE_UNAVAILABLE;
    };

    let target = req.room_id.as_deref()
        .unwrap_or(state.cfg.matrix.room_id.as_str());

    let result = match req.format {
        NotifyFormat::Plain => bot.send_to_room(target, &req.message).await,
        NotifyFormat::Markdown => bot.send_markdown_to_room(target, &req.message).await,
    };

    match result {
        Ok(()) => {
            tracing::info!(room = target, "notify: message sent to matrix");
            StatusCode::OK
        }
        Err(e) => {
            tracing::error!(%e, room = target, "notify: failed to send to matrix");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}
