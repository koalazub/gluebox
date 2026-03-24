use std::collections::HashMap;
use std::path::Path;
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;

use super::char_sessions::SessionMeta;

#[derive(Debug, Clone, Deserialize)]
pub struct CharCalendar {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CharEvent {
    pub id: String,
    pub title: String,
    pub calendar_id: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct MatchedEvent {
    pub event_id: String,
    pub event_title: String,
    pub calendar_name: String,
}

pub fn load_calendars(hyprnote_dir: &Path) -> HashMap<String, CharCalendar> {
    let path = hyprnote_dir.join("calendars.json");
    let Ok(content) = std::fs::read_to_string(&path) else {
        return HashMap::new();
    };
    let calendars: Vec<CharCalendar> = serde_json::from_str(&content).unwrap_or_default();
    calendars.into_iter().map(|c| (c.id.clone(), c)).collect()
}

pub fn load_events(hyprnote_dir: &Path) -> HashMap<String, CharEvent> {
    let path = hyprnote_dir.join("events.json");
    let Ok(content) = std::fs::read_to_string(&path) else {
        return HashMap::new();
    };
    let events: Vec<CharEvent> = serde_json::from_str(&content).unwrap_or_default();
    events.into_iter().map(|e| (e.id.clone(), e)).collect()
}

pub fn find_uni_calendar_ids(
    calendars: &HashMap<String, CharCalendar>,
    uni_names: &[String],
) -> Vec<String> {
    calendars
        .values()
        .filter(|c| uni_names.iter().any(|n| c.name.eq_ignore_ascii_case(n)))
        .map(|c| c.id.clone())
        .collect()
}

pub fn match_session_to_event(
    meta: &SessionMeta,
    events: &HashMap<String, CharEvent>,
    calendars: &HashMap<String, CharCalendar>,
    uni_calendar_ids: &[String],
) -> Option<MatchedEvent> {
    let grace = Duration::minutes(15);
    let session_time = meta.created_at;

    let mut best_match: Option<(MatchedEvent, i64)> = None;

    for event in events.values() {
        if !uni_calendar_ids.contains(&event.calendar_id) {
            continue;
        }

        let window_start = event.started_at - grace;
        let window_end = event.ended_at + grace;

        if session_time >= window_start && session_time <= window_end {
            let mid = event.started_at + (event.ended_at - event.started_at) / 2;
            let distance = (session_time - mid).num_seconds().unsigned_abs() as i64;

            let calendar_name = calendars
                .get(&event.calendar_id)
                .map(|c| c.name.clone())
                .unwrap_or_default();

            let candidate = MatchedEvent {
                event_id: event.id.clone(),
                event_title: event.title.clone(),
                calendar_name,
            };

            match &best_match {
                Some((_, best_distance)) if distance < *best_distance => {
                    best_match = Some((candidate, distance));
                }
                None => {
                    best_match = Some((candidate, distance));
                }
                _ => {}
            }
        }
    }

    best_match.map(|(m, _)| m)
}
