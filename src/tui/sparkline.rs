use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

const BLOCK_CHARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

pub(super) fn render_connector_row(
    info: &super::ConnectorInfo,
    selected: bool,
    frame_count: u64,
) -> Line<'static> {
    let spans = vec![
        Span::raw(" "),
        status_icon_span(&info.status, frame_count),
        Span::raw(" "),
        name_span(&info.name, selected),
        Span::raw(" "),
        sparkline_span(&info.sparkline),
        Span::raw(" "),
        event_count_span(info.event_count),
    ];

    let row_style = if selected {
        Style::default().bg(Color::Rgb(40, 40, 60))
    } else {
        Style::default()
    };

    Line::from(spans).style(row_style)
}

fn status_icon_span(status: &str, frame_count: u64) -> Span<'static> {
    match status {
        "running" => {
            let color = if frame_count % 10 < 5 {
                Color::LightGreen
            } else {
                Color::Green
            };
            Span::styled("●", Style::default().fg(color))
        }
        "suspended" => Span::styled("◐", Style::default().fg(Color::Yellow)),
        "stopped" => Span::styled("○", Style::default().fg(Color::DarkGray)),
        "error" => {
            let color = if frame_count % 5 < 2 {
                Color::Red
            } else {
                Color::Rgb(100, 0, 0)
            };
            Span::styled("✖", Style::default().fg(color))
        }
        _ => Span::styled("?", Style::default().fg(Color::DarkGray)),
    }
}

fn name_span(name: &str, selected: bool) -> Span<'static> {
    let formatted = format!("{:<14}", name);
    let style = if selected {
        Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };
    Span::styled(formatted, style)
}

fn sparkline_span(data: &[u8]) -> Span<'static> {
    let chars: String = data
        .iter()
        .map(|&v| {
            let idx = (v as usize * 7) / 255;
            BLOCK_CHARS[idx]
        })
        .collect();
    Span::styled(chars, Style::default().fg(Color::Cyan).add_modifier(Modifier::DIM))
}

fn event_count_span(count: u64) -> Span<'static> {
    Span::styled(
        format!("{:>6} evt", count),
        Style::default().fg(Color::DarkGray),
    )
}
