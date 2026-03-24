use std::sync::Arc;
use crate::AppState;
use crate::db::SessionImport;
use crate::connectors::char_sessions;
use crate::connectors::calendar;
use super::affine_from_registry;

pub async fn import_session(state: &Arc<AppState>, session_id: &str) -> anyhow::Result<String> {
    let config = state.config.read().await;
    let watcher_cfg = config.watcher.as_ref()
        .ok_or_else(|| anyhow::anyhow!("watcher config not set"))?;

    let session_dir = watcher_cfg.sessions_dir.join(session_id);
    let parsed = char_sessions::parse_session(&session_dir)?;

    let calendars = calendar::load_calendars(&watcher_cfg.hyprnote_dir);
    let events = calendar::load_events(&watcher_cfg.hyprnote_dir);
    let uni_ids = calendar::find_uni_calendar_ids(&calendars, &watcher_cfg.uni_calendar_names);

    let matched = calendar::match_session_to_event(&parsed.meta, &events, &calendars, &uni_ids);

    let calendar_name = matched.as_ref().map(|m| m.calendar_name.clone());
    let event_title = matched.as_ref().map(|m| m.event_title.clone());

    drop(config);

    let affine_doc_id = match affine_from_registry(state).await {
        Ok(client) => {
            let title = if let Some(ref evt) = event_title {
                format!("{} - {}", evt, parsed.meta.title)
            } else {
                parsed.meta.title.clone()
            };
            match client.create_document(&title, &parsed.summary).await {
                Ok(id) => Some(id),
                Err(e) => {
                    tracing::error!("failed to create affine doc: {e}");
                    None
                }
            }
        }
        Err(_) => None,
    };

    let import = SessionImport {
        session_id: session_id.to_string(),
        session_title: parsed.meta.title.clone(),
        affine_doc_id: affine_doc_id.clone(),
        calendar_name,
        event_title,
        imported_at: None,
    };
    state.db.upsert_import(&import).await?;

    let result = format!(
        "imported session '{}' (affine_doc={})",
        parsed.meta.title,
        affine_doc_id.as_deref().unwrap_or("none"),
    );
    tracing::info!("{result}");
    Ok(result)
}

pub async fn import_latest_uni(state: &Arc<AppState>) -> anyhow::Result<String> {
    let config = state.config.read().await;
    let watcher_cfg = config.watcher.as_ref()
        .ok_or_else(|| anyhow::anyhow!("watcher config not set"))?;

    let dirs = char_sessions::list_session_dirs(&watcher_cfg.sessions_dir);
    let calendars = calendar::load_calendars(&watcher_cfg.hyprnote_dir);
    let events = calendar::load_events(&watcher_cfg.hyprnote_dir);
    let uni_ids = calendar::find_uni_calendar_ids(&calendars, &watcher_cfg.uni_calendar_names);

    let mut latest: Option<(char_sessions::ParsedSession, calendar::MatchedEvent)> = None;

    for dir in &dirs {
        let Ok(parsed) = char_sessions::parse_session(dir) else { continue };
        if let Some(matched) = calendar::match_session_to_event(&parsed.meta, &events, &calendars, &uni_ids) {
            match &latest {
                Some((existing, _)) if parsed.meta.created_at > existing.meta.created_at => {
                    latest = Some((parsed, matched));
                }
                None => {
                    latest = Some((parsed, matched));
                }
                _ => {}
            }
        }
    }

    drop(config);

    let (session, _) = latest.ok_or_else(|| anyhow::anyhow!("no uni sessions found"))?;
    let session_id = session.meta.id.clone();
    import_session(state, &session_id).await
}

pub async fn import_all_uni(state: &Arc<AppState>) -> anyhow::Result<String> {
    let config = state.config.read().await;
    let watcher_cfg = config.watcher.as_ref()
        .ok_or_else(|| anyhow::anyhow!("watcher config not set"))?;

    let dirs = char_sessions::list_session_dirs(&watcher_cfg.sessions_dir);
    let calendars = calendar::load_calendars(&watcher_cfg.hyprnote_dir);
    let events = calendar::load_events(&watcher_cfg.hyprnote_dir);
    let uni_ids = calendar::find_uni_calendar_ids(&calendars, &watcher_cfg.uni_calendar_names);

    let mut session_ids: Vec<String> = Vec::new();
    for dir in &dirs {
        let Ok(parsed) = char_sessions::parse_session(dir) else { continue };
        if calendar::match_session_to_event(&parsed.meta, &events, &calendars, &uni_ids).is_some() {
            if !state.db.is_imported(&parsed.meta.id).await.unwrap_or(true) {
                session_ids.push(parsed.meta.id);
            }
        }
    }

    drop(config);

    let count = session_ids.len();
    for sid in &session_ids {
        if let Err(e) = import_session(state, sid).await {
            tracing::error!(session_id = sid.as_str(), "import failed: {e}");
        }
    }

    Ok(format!("imported {count} uni sessions"))
}
