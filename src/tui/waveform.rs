use ratatui::prelude::*;
use ratatui::widgets::Widget;

const WAVEFORM_CHARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
const BUFFER_SIZE: usize = 60;

pub struct WaveformState {
    buffer: Vec<f64>,
    threshold: f64,
}

impl WaveformState {
    pub fn new() -> Self {
        Self {
            buffer: Vec::with_capacity(BUFFER_SIZE),
            threshold: 5.0,
        }
    }

    pub fn push(&mut self, potential: f64, threshold: f64) {
        self.threshold = threshold;
        if self.buffer.len() >= BUFFER_SIZE {
            self.buffer.remove(0);
        }
        self.buffer.push(potential);
    }
}

pub struct WaveformWidget<'a> {
    state: &'a WaveformState,
}

impl<'a> WaveformWidget<'a> {
    pub fn new(state: &'a WaveformState) -> Self {
        Self { state }
    }
}

impl Widget for WaveformWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let width = area.width as usize;
        let ceiling = self.state.threshold * 1.5;

        let values: Vec<f64> = self.state.buffer
            .iter()
            .rev()
            .take(width)
            .copied()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();

        for (i, &value) in values.iter().enumerate() {
            let normalized = (value / ceiling).clamp(0.0, 1.0);
            let char_index = (normalized * 7.0).round() as usize;
            let char_index = char_index.min(7);
            let waveform_char = WAVEFORM_CHARS[char_index];

            let color = if normalized < 0.25 {
                Color::Rgb(30, 60, 120)
            } else if normalized < 0.50 {
                Color::Rgb(0, 180, 200)
            } else if normalized < 0.75 {
                Color::Rgb(0, 200, 80)
            } else if normalized < 0.90 {
                Color::Rgb(255, 255, 100)
            } else {
                Color::Rgb(255, 160, 50)
            };

            let x = area.x + i as u16;
            let y = area.y;
            if x < area.x + area.width && y < area.y + area.height {
                buf[(x, y)]
                    .set_char(waveform_char)
                    .set_fg(color);
            }
        }

        if area.height > 1 && self.state.threshold > 0.0 {
            let ceiling = self.state.threshold * 1.5;
            let threshold_norm = (self.state.threshold / ceiling).clamp(0.0, 1.0);
            let threshold_row = area.height.saturating_sub(1)
                - (threshold_norm * (area.height - 1) as f64).round() as u16;
            let threshold_y = area.y + threshold_row;

            if threshold_y < area.y + area.height {
                for col in 0..area.width {
                    let x = area.x + col;
                    let cell = &mut buf[(x, threshold_y)];
                    if cell.symbol() == " " {
                        cell.set_char('╌')
                            .set_fg(Color::DarkGray)
                            .set_style(Style::default().add_modifier(Modifier::DIM));
                    }
                }
            }
        }
    }
}
