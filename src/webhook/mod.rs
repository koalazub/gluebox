mod verify;

use std::sync::Arc;
use axum::{
    Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use bytes::Bytes;

use crate::AppState;
use crate::triggers;

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/webhooks/linear", post(handle_linear))
        .route("/webhooks/documenso", post(handle_documenso))
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
        Some(&body.iter().map(|&b| b as char).collect::<String>())) {
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
        &payload.payload.id.to_string(), None) {
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
