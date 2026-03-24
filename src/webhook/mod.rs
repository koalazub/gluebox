mod verify;

use std::sync::Arc;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use bytes::Bytes;
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::triggers;
use crate::triggers::to_matrix;
use crate::triggers::opencode_from_registry;
use crate::openclaw::process_feedback_clusters;

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/webhooks/linear", post(handle_linear))
        .route("/webhooks/documenso", post(handle_documenso))
        .route("/webhooks/github", post(handle_github))
        .route("/api/notify", post(handle_notify))
        .route("/api/feedback", post(handle_feedback))
        .route("/admin/status", get(admin_status))
        .route("/admin/connectors", get(admin_connectors))
        .route("/admin/connectors/{name}/toggle", post(admin_toggle))
        .route("/admin/reload", post(admin_reload))
        .route("/admin/spike", post(admin_spike))
        .route("/admin/power", get(admin_power))
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
    state.power.spike();
    let webhook_secret = {
        let cfg = state.config.read().await;
        let Some(ref linear_cfg) = cfg.linear else {
            return StatusCode::SERVICE_UNAVAILABLE;
        };
        linear_cfg.webhook_secret.clone()
    };

    let signature = headers
        .get("linear-signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !verify::linear_signature(signature, &body, &webhook_secret) {
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
    state.power.spike();
    let gh_webhook_secret = {
        let cfg = state.config.read().await;
        let Some(ref gh_cfg) = cfg.github else {
            return StatusCode::NOT_FOUND;
        };
        gh_cfg.webhook_secret.clone()
    };

    let signature = headers
        .get("x-hub-signature-256")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !verify::github_signature(signature, &body, &gh_webhook_secret) {
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
    state.power.spike();
    let secret = headers
        .get("x-documenso-secret")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let documenso_webhook_secret = {
        let cfg = state.config.read().await;
        let Some(ref documenso_cfg) = cfg.documenso else {
            return StatusCode::SERVICE_UNAVAILABLE;
        };
        documenso_cfg.webhook_secret.clone()
    };

    if !verify::documenso_secret(secret, &documenso_webhook_secret) {
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
    state.power.spike();
    let notify_secret = {
        let cfg = state.config.read().await;
        let Some(ref secret) = cfg.notify_secret else {
            return StatusCode::NOT_FOUND;
        };
        secret.clone()
    };

    let provided = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");

    if !verify::constant_time_eq_pub(provided.as_bytes(), notify_secret.as_bytes()) {
        return StatusCode::UNAUTHORIZED;
    }

    let ai = match opencode_from_registry(&state).await {
        Ok(c) => Arc::new(c),
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE,
    };

    tokio::spawn(async move {

        let mut llm_input = req.message.clone();
        
        if !req.category.is_empty() {
            llm_input = format!("[Category: {}]\n\n{}", req.category, llm_input);
        }
        
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
    state.power.spike();
    let (notify_secret, default_room_id) = {
        let cfg = state.config.read().await;
        let Some(ref secret) = cfg.notify_secret else {
            tracing::warn!("notify endpoint called but notify_secret not configured");
            return StatusCode::NOT_FOUND;
        };
        let Some(ref matrix_cfg) = cfg.matrix else {
            tracing::error!("notify endpoint: matrix not configured");
            return StatusCode::SERVICE_UNAVAILABLE;
        };
        (secret.clone(), matrix_cfg.room_id.clone())
    };

    let provided = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");

    if !verify::constant_time_eq_pub(provided.as_bytes(), notify_secret.as_bytes()) {
        tracing::warn!("notify endpoint: invalid bearer token");
        return StatusCode::UNAUTHORIZED;
    }

    let matrix_conn = match state.registry.get_dyn("matrix").await {
        Some(c) => c,
        None => {
            tracing::error!("notify endpoint: matrix connector not registered");
            return StatusCode::SERVICE_UNAVAILABLE;
        }
    };
    let matrix_connector = match matrix_conn.as_any().downcast_ref::<crate::connectors::matrix::MatrixConnector>() {
        Some(c) => c,
        None => {
            tracing::error!("notify endpoint: registry 'matrix' is not MatrixConnector");
            return StatusCode::SERVICE_UNAVAILABLE;
        }
    };
    let bot = match matrix_connector.bot().await {
        Ok(b) => b,
        Err(_) => {
            tracing::error!("notify endpoint: matrix bot not running");
            return StatusCode::SERVICE_UNAVAILABLE;
        }
    };

    let target = req.room_id.as_deref()
        .unwrap_or(default_room_id.as_str());

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

async fn check_admin_auth(state: &Arc<AppState>, headers: &HeaderMap) -> Result<(), StatusCode> {
    let cfg = state.config.read().await;
    let secret = cfg.notify_secret.as_deref().ok_or(StatusCode::FORBIDDEN)?;
    let provided = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or(StatusCode::UNAUTHORIZED)?;
    if provided != secret {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(())
}

#[derive(Serialize)]
struct ConnectorInfo {
    name: String,
    status: String,
}

#[derive(Serialize)]
struct AdminStatusResponse {
    power_state: String,
    potential: f64,
    threshold: f64,
    uptime_secs: u64,
    connectors: Vec<ConnectorInfo>,
    events_per_min: f64,
}

#[derive(Serialize)]
struct AdminPowerResponse {
    state: String,
    potential: f64,
    threshold: f64,
}

fn connector_status_str(status: &crate::connector::ConnectorStatus) -> String {
    match status {
        crate::connector::ConnectorStatus::Running => "running".to_string(),
        crate::connector::ConnectorStatus::Stopped => "stopped".to_string(),
        crate::connector::ConnectorStatus::Suspended => "suspended".to_string(),
        crate::connector::ConnectorStatus::Error(e) => format!("error: {e}"),
    }
}

fn power_state_str(state: &crate::power::PowerState) -> String {
    match state {
        crate::power::PowerState::Active => "active".to_string(),
        crate::power::PowerState::Resting => "resting".to_string(),
    }
}

async fn admin_status(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<AdminStatusResponse>, StatusCode> {
    check_admin_auth(&state, &headers).await?;

    let connectors = state
        .registry
        .list()
        .await
        .into_iter()
        .map(|(name, status)| ConnectorInfo {
            name,
            status: connector_status_str(&status),
        })
        .collect();

    Ok(Json(AdminStatusResponse {
        power_state: power_state_str(&state.power.state()),
        potential: state.power.potential(),
        threshold: state.power.threshold(),
        uptime_secs: state.started_at.elapsed().as_secs(),
        connectors,
        events_per_min: 0.0,
    }))
}

async fn admin_connectors(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Vec<ConnectorInfo>>, StatusCode> {
    check_admin_auth(&state, &headers).await?;

    let connectors = state
        .registry
        .list()
        .await
        .into_iter()
        .map(|(name, status)| ConnectorInfo {
            name,
            status: connector_status_str(&status),
        })
        .collect();

    Ok(Json(connectors))
}

async fn admin_toggle(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> Result<Json<ConnectorInfo>, StatusCode> {
    check_admin_auth(&state, &headers).await?;

    let new_status = state
        .registry
        .toggle(&name)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;

    Ok(Json(ConnectorInfo {
        name,
        status: connector_status_str(&new_status),
    }))
}

async fn admin_reload(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, StatusCode> {
    check_admin_auth(&state, &headers).await?;
    match crate::daemon::reload(&state).await {
        Ok(msg) => Ok(Json(serde_json::json!({"result": msg}))),
        Err(e) => {
            tracing::error!(%e, "admin reload failed");
            Ok(Json(serde_json::json!({"error": e.to_string()})))
        }
    }
}

async fn admin_spike(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, StatusCode> {
    check_admin_auth(&state, &headers).await?;
    state.power.spike();
    Ok(Json(serde_json::json!({"result": "spike sent"})))
}

async fn admin_power(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<AdminPowerResponse>, StatusCode> {
    check_admin_auth(&state, &headers).await?;
    Ok(Json(AdminPowerResponse {
        state: power_state_str(&state.power.state()),
        potential: state.power.potential(),
        threshold: state.power.threshold(),
    }))
}
