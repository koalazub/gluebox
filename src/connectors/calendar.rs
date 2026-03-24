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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn make_calendar(name: &str) -> CharCalendar {
        CharCalendar {
            name: name.to_string(),
            enabled: true,
            provider: "test".to_string(),
            source: "test-source".to_string(),
        }
    }

    fn make_event(calendar_id: &str, title: &str, start: DateTime<Utc>, end: DateTime<Utc>) -> CharEvent {
        CharEvent {
            calendar_id: calendar_id.to_string(),
            title: title.to_string(),
            started_at: start,
            ended_at: end,
            description: String::new(),
            location: String::new(),
        }
    }

    fn make_session(id: &str, created_at: DateTime<Utc>) -> SessionMeta {
        SessionMeta {
            id: id.to_string(),
            title: format!("Session {id}"),
            created_at,
        }
    }

    fn fixed_time(hour: u32, min: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 3, 24, hour, min, 0).unwrap()
    }

    #[test]
    fn find_uni_calendar_ids_matches_case_insensitive() {
        let mut calendars = CalendarsMap::new();
        calendars.insert("cal-uni".into(), make_calendar("Uni"));
        calendars.insert("cal-home".into(), make_calendar("Home"));

        let uni_names = vec!["uni".to_string()];
        let ids = find_uni_calendar_ids(&calendars, &uni_names);

        assert_eq!(ids.len(), 1);
        assert!(ids.contains(&"cal-uni".to_string()));
    }

    #[test]
    fn match_session_to_event_within_window() {
        let mut calendars = CalendarsMap::new();
        calendars.insert("cal-uni".into(), make_calendar("Uni"));

        let mut events = EventsMap::new();
        events.insert("evt-1".into(), make_event("cal-uni", "CS101", fixed_time(1, 0), fixed_time(3, 0)));

        let session = make_session("s1", fixed_time(1, 30));
        let uni_ids = vec!["cal-uni".to_string()];

        let matched = match_session_to_event(&session, &events, &calendars, &uni_ids);
        assert!(matched.is_some());
        let m = matched.unwrap();
        assert_eq!(m.event_title, "CS101");
        assert_eq!(m.calendar_name, "Uni");
    }

    #[test]
    fn match_session_to_event_outside_window() {
        let mut calendars = CalendarsMap::new();
        calendars.insert("cal-uni".into(), make_calendar("Uni"));

        let mut events = EventsMap::new();
        events.insert("evt-1".into(), make_event("cal-uni", "CS101", fixed_time(1, 0), fixed_time(3, 0)));

        let session = make_session("s2", fixed_time(10, 0));
        let uni_ids = vec!["cal-uni".to_string()];

        let matched = match_session_to_event(&session, &events, &calendars, &uni_ids);
        assert!(matched.is_none());
    }

    #[test]
    fn match_session_to_event_within_grace_period() {
        let mut calendars = CalendarsMap::new();
        calendars.insert("cal-uni".into(), make_calendar("Uni"));

        let mut events = EventsMap::new();
        events.insert("evt-1".into(), make_event("cal-uni", "CS101", fixed_time(2, 0), fixed_time(3, 0)));

        let session = make_session("s3", fixed_time(1, 50));
        let uni_ids = vec!["cal-uni".to_string()];

        let matched = match_session_to_event(&session, &events, &calendars, &uni_ids);
        assert!(matched.is_some());
        assert_eq!(matched.unwrap().event_title, "CS101");
    }

    #[test]
    fn match_session_to_event_skips_non_uni() {
        let mut calendars = CalendarsMap::new();
        calendars.insert("cal-home".into(), make_calendar("Home"));
        calendars.insert("cal-uni".into(), make_calendar("Uni"));

        let mut events = EventsMap::new();
        events.insert("evt-home".into(), make_event("cal-home", "Dentist", fixed_time(1, 0), fixed_time(2, 0)));

        let session = make_session("s4", fixed_time(1, 30));
        let uni_ids = vec!["cal-uni".to_string()];

        let matched = match_session_to_event(&session, &events, &calendars, &uni_ids);
        assert!(matched.is_none());
    }

    #[test]
    fn match_session_to_event_picks_closest() {
        let mut calendars = CalendarsMap::new();
        calendars.insert("cal-uni".into(), make_calendar("Uni"));

        let mut events = EventsMap::new();
        events.insert("evt-far".into(), make_event("cal-uni", "Far Lecture", fixed_time(1, 0), fixed_time(4, 0)));
        events.insert("evt-close".into(), make_event("cal-uni", "Close Lecture", fixed_time(2, 0), fixed_time(3, 0)));

        let session = make_session("s5", fixed_time(2, 5));
        let uni_ids = vec!["cal-uni".to_string()];

        let matched = match_session_to_event(&session, &events, &calendars, &uni_ids);
        assert!(matched.is_some());
        assert_eq!(matched.unwrap().event_title, "Close Lecture");
    }
}
