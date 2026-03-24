use std::time::Duration;

use ratatui::prelude::Stylize;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

pub(super) fn render_event_line(
    entry: &super::EventEntry,
    now: std::time::Instant,
) -> Line<'static> {
    let age = now.duration_since(entry.received_at);
    let line_style = if age > Duration::from_secs(5) {
        Style::new().dim()
    } else {
        Style::new()
    };

    let source_color = match entry.source.as_str() {
        "linear" => Color::Rgb(130, 80, 200),
        "matrix" => Color::Rgb(80, 200, 120),
        "github" => Color::White,
        "documenso" => Color::Rgb(80, 140, 220),
        "anytype" => Color::Rgb(0, 200, 200),
        "opencode" => Color::Rgb(220, 200, 80),
        _ => Color::Gray,
    };

    let source_tag = Span::styled(
        format!("{:<10}", entry.source),
        Style::new().fg(source_color),
    );

    let event_detail = Span::raw(format!("{} {}", entry.event_type, entry.detail));

    Line::from(vec![source_tag, event_detail]).style(line_style)
}
