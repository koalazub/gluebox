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
use crate::connectors::opencode::OpenCodeClient;
use crate::openclaw::process_feedback_clusters;

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/webhooks/linear", post(handle_linear))
        .route("/webhooks/documenso", post(handle_documenso))
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
        ("Issue", "create") => triggers::linear_issue_created(&state, &payload).await,
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

#[derive(Deserialize)]
struct FeedbackRequest {
    message: String,
}

async fn handle_feedback(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<FeedbackRequest>,
) -> (StatusCode, axum::Json<serde_json::Value>) {
    let Some(ref secret) = state.cfg.notify_secret else {
        return (StatusCode::NOT_FOUND, axum::Json(serde_json::json!({"error": "not configured"})));
    };

    let provided = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");

    if !verify::constant_time_eq_pub(provided.as_bytes(), secret.as_bytes()) {
        return (StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({"error": "unauthorized"})));
    }

    let Some(ref opencode_cfg) = state.cfg.opencode else {
        return (StatusCode::SERVICE_UNAVAILABLE, axum::Json(serde_json::json!({"error": "opencode not configured"})));
    };

    let ai = std::sync::Arc::new(OpenCodeClient::new(&opencode_cfg.api_key));

    let clusters = match ai.extract_and_cluster_feedback(&req.message).await {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "feedback api: failed to cluster feedback");
            return (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(serde_json::json!({"error": e.to_string()})));
        }
    };

    if clusters.is_empty() {
        return (StatusCode::OK, axum::Json(serde_json::json!({
            "clusters_processed": 0,
            "results": []
        })));
    }

    let results = process_feedback_clusters(&state, &ai, &clusters).await;

    (StatusCode::OK, axum::Json(serde_json::json!({
        "clusters_processed": clusters.len(),
        "results": results,
    })))
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
