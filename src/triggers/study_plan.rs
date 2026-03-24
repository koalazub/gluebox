use std::sync::Arc;
use crate::AppState;
use crate::connectors::char_sessions;
use crate::connectors::calendar;
use super::affine_from_registry;

pub async fn generate_plan(state: &Arc<AppState>, period: &str, course: Option<&str>) -> anyhow::Result<String> {
    let config = state.config.read().await;
    let watcher_cfg = config.watcher.as_ref()
        .ok_or_else(|| anyhow::anyhow!("watcher config not set"))?;

    let dirs = char_sessions::list_session_dirs(&watcher_cfg.sessions_dir);
    let calendars = calendar::load_calendars(&watcher_cfg.hyprnote_dir);
    let events = calendar::load_events(&watcher_cfg.hyprnote_dir);
    let uni_ids = calendar::find_uni_calendar_ids(&calendars, &watcher_cfg.uni_calendar_names);

    let mut lectures: Vec<(String, String)> = Vec::new();

    for dir in &dirs {
        let Ok(parsed) = char_sessions::parse_session(dir) else { continue };
        if let Some(matched) = calendar::match_session_to_event(&parsed.meta, &events, &calendars, &uni_ids) {
            if let Some(filter_course) = course {
                if !matched.event_title.to_lowercase().contains(&filter_course.to_lowercase()) {
                    continue;
                }
            }
            lectures.push((matched.event_title, parsed.summary.unwrap_or_default()));
        }
    }

    drop(config);

    let mut markdown = format!("# Study Plan - {period}\n\n");

    if lectures.is_empty() {
        markdown.push_str("No lecture sessions found for this period.\n");
    } else {
        markdown.push_str(&format!("## Lectures ({} sessions)\n\n", lectures.len()));
        for (title, summary) in &lectures {
            markdown.push_str(&format!("### {title}\n\n"));
            if !summary.is_empty() {
                let preview = if summary.len() > 500 {
                    format!("{}...", &summary[..500])
                } else {
                    summary.clone()
                };
                markdown.push_str(&format!("{preview}\n\n"));
            }
        }

        markdown.push_str("## Review Schedule\n\n");
        markdown.push_str("- [ ] Review lecture notes within 24 hours\n");
        markdown.push_str("- [ ] Create summary flashcards\n");
        markdown.push_str("- [ ] Practice problems / exercises\n");
        markdown.push_str("- [ ] Spaced review at 3 days, 1 week, 2 weeks\n");
    }

    let affine_doc_id = match affine_from_registry(state).await {
        Ok(client) => {
            let title = format!("Study Plan - {period}");
            match client.create_document(&title, &markdown).await {
                Ok(id) => Some(id),
                Err(e) => {
                    tracing::error!("failed to create affine study plan doc: {e}");
                    None
                }
            }
        }
        Err(_) => None,
    };

    state.db.insert_study_plan(period, affine_doc_id.as_deref()).await?;

    let result = format!(
        "study plan created for '{}' with {} lectures (affine_doc={})",
        period,
        lectures.len(),
        affine_doc_id.as_deref().unwrap_or("none"),
    );
    tracing::info!("{result}");
    Ok(result)
}
