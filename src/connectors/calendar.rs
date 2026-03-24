use std::collections::HashMap;
use std::path::Path;
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;

use super::char_sessions::SessionMeta;

#[derive(Debug, Clone, Deserialize)]
pub struct CharCalendar {
    pub name: String,
    pub enabled: bool,
    pub provider: String,
    pub source: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CharEvent {
    pub calendar_id: String,
    pub title: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub location: String,
}

#[derive(Debug, Clone)]
pub struct MatchedEvent {
    pub event_id: String,
    pub event_title: String,
    pub calendar_name: String,
}

pub type CalendarsMap = HashMap<String, CharCalendar>;
pub type EventsMap = HashMap<String, CharEvent>;

pub fn load_calendars(hyprnote_dir: &Path) -> CalendarsMap {
    let path = hyprnote_dir.join("calendars.json");
    let Ok(content) = std::fs::read_to_string(&path) else {
        return HashMap::new();
    };
    serde_json::from_str(&content).unwrap_or_default()
}

pub fn load_events(hyprnote_dir: &Path) -> EventsMap {
    let path = hyprnote_dir.join("events.json");
    let Ok(content) = std::fs::read_to_string(&path) else {
        return HashMap::new();
    };
    serde_json::from_str(&content).unwrap_or_default()
}

pub fn find_uni_calendar_ids(
    calendars: &CalendarsMap,
    uni_names: &[String],
) -> Vec<String> {
    calendars
        .iter()
        .filter(|(_, cal)| uni_names.iter().any(|n| cal.name.eq_ignore_ascii_case(n)))
        .map(|(id, _)| id.clone())
        .collect()
}

pub fn match_session_to_event(
    meta: &SessionMeta,
    events: &EventsMap,
    calendars: &CalendarsMap,
    uni_calendar_ids: &[String],
) -> Option<MatchedEvent> {
    let grace = Duration::minutes(15);
    let session_time = meta.created_at;

    let mut best: Option<(String, &CharEvent, i64)> = None;

    for (event_id, event) in events {
        if !uni_calendar_ids.contains(&event.calendar_id) {
            continue;
        }

        let window_start = event.started_at - grace;
        let window_end = event.ended_at + grace;

        if session_time >= window_start && session_time <= window_end {
            let distance = (session_time - event.started_at).num_seconds().abs();
            match &best {
                Some((_, _, best_dist)) if distance < *best_dist => {
                    best = Some((event_id.clone(), event, distance));
                }
                None => {
                    best = Some((event_id.clone(), event, distance));
                }
                _ => {}
            }
        }
    }

    best.map(|(event_id, event, _)| {
        let calendar_name = calendars
            .get(&event.calendar_id)
            .map(|c| c.name.clone())
            .unwrap_or_default();

        MatchedEvent {
            event_id,
            event_title: event.title.clone(),
            calendar_name,
        }
    })
}
