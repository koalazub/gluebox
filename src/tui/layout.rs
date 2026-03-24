use std::time::Instant;

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use super::event_feed;
use super::sparkline;
use super::waveform::WaveformWidget;
use super::TuiState;

pub fn render(state: &TuiState, frame: &mut Frame) {
    let area = frame.area();

    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(1),
        ])
        .split(area);

    render_power_bar(state, frame, vertical[0]);
    render_middle(state, frame, vertical[1]);
    render_help_bar(frame, vertical[2]);
}

fn render_power_bar(state: &TuiState, frame: &mut Frame, area: Rect) {
    let power_color = match state.power_state.as_str() {
        "Active" => Color::Green,
        "Resting" => Color::Yellow,
        _ => Color::DarkGray,
    };

    let block = Block::default()
        .title(" GLUEBOX ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Blue));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let stats_text = {
        let uptime_minutes = state.uptime_secs / 60;
        let uptime_seconds = state.uptime_secs % 60;
        format!(
            " {} {:>5.1}/{:.1}  {:.0} evt/min  {:02}:{:02}",
            state.power_state.to_uppercase(),
            state.potential,
            state.threshold,
            state.events_per_min,
            uptime_minutes,
            uptime_seconds,
        )
    };
    let stats_width = stats_text.len() as u16;

    let waveform_width = inner.width.saturating_sub(stats_width);

    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(waveform_width),
            Constraint::Length(stats_width),
        ])
        .split(inner);

    let waveform_widget = WaveformWidget::new(&state.waveform);
    frame.render_widget(waveform_widget, horizontal[0]);

    let power_indicator = Span::styled(
        format!(" {} ", state.power_state.to_uppercase()),
        Style::default().fg(Color::Black).bg(power_color).add_modifier(Modifier::BOLD),
    );

    let uptime_minutes = state.uptime_secs / 60;
    let uptime_seconds = state.uptime_secs % 60;

    let status_line = Line::from(vec![
        Span::raw(" "),
        power_indicator,
        Span::raw(" "),
        Span::styled(
            format!("{:.1}/{:.1}", state.potential, state.threshold),
            Style::default().fg(Color::Cyan),
        ),
        Span::raw("  "),
        Span::styled(
            format!("{:.0} evt/min", state.events_per_min),
            Style::default().fg(Color::Magenta),
        ),
        Span::raw("  "),
        Span::styled(
            format!("{:02}:{:02}", uptime_minutes, uptime_seconds),
            Style::default().fg(Color::DarkGray),
        ),
    ]);

    let paragraph = Paragraph::new(status_line);
    frame.render_widget(paragraph, horizontal[1]);
}

fn render_middle(state: &TuiState, frame: &mut Frame, area: Rect) {
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    render_connectors(state, frame, horizontal[0]);
    render_events(state, frame, horizontal[1]);
}

fn render_connectors(state: &TuiState, frame: &mut Frame, area: Rect) {
    let block = Block::default()
        .title(" Connectors ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Blue));

    let items: Vec<ListItem> = state
        .connectors
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let selected = i == state.selected_connector;
            let line = sparkline::render_connector_row(c, selected, state.frame_count);
            ListItem::new(line)
        })
        .collect();

    let mut list_state = ListState::default();
    if !state.connectors.is_empty() {
        list_state.select(Some(state.selected_connector));
    }

    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .bg(Color::Rgb(40, 40, 60))
                .add_modifier(Modifier::BOLD),
        );

    frame.render_stateful_widget(list, area, &mut list_state);
}

fn render_events(state: &TuiState, frame: &mut Frame, area: Rect) {
    let block = Block::default()
        .title(" Events ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Blue));

    let inner_height = area.height.saturating_sub(2) as usize;
    let skip = state.events.len().saturating_sub(inner_height);

    let now = Instant::now();
    let items: Vec<ListItem> = state
        .events
        .iter()
        .skip(skip)
        .map(|e| {
            let line = event_feed::render_event_line(e, now);
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items).block(block);
    frame.render_widget(list, area);
}

fn render_help_bar(frame: &mut Frame, area: Rect) {
    let help = Line::from(vec![
        Span::raw("  "),
        Span::styled("[t]", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw("oggle  "),
        Span::styled("[r]", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw("eload  "),
        Span::styled("[q]", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw("uit  "),
        Span::styled("[↑↓]", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw(" select"),
    ]);

    let paragraph = Paragraph::new(help);
    frame.render_widget(paragraph, area);
}
