mod layout;
pub mod event_feed;
pub mod sparkline;
pub mod waveform;

use std::io::{Read as _, Write as _};
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::ExecutableCommand as _;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::gluebox_capnp;
use crate::socket;

struct TuiState {
    connectors: Vec<ConnectorInfo>,
    potential: f64,
    threshold: f64,
    power_state: String,
    events_per_min: f32,
    uptime_secs: u64,
    framerate: u8,
    events: Vec<EventEntry>,
    selected_connector: usize,
    waveform: waveform::WaveformState,
    frame_count: u64,
}

#[allow(dead_code)]
struct ConnectorInfo {
    name: String,
    status: String,
    sparkline: Vec<u8>,
    event_count: u64,
    error_message: String,
}

struct EventEntry {
    source: String,
    event_type: String,
    detail: String,
    received_at: Instant,
}

impl TuiState {
    fn new() -> Self {
        Self {
            connectors: Vec::new(),
            potential: 0.0,
            threshold: 5.0,
            power_state: "Unknown".into(),
            events_per_min: 0.0,
            uptime_secs: 0,
            framerate: 10,
            events: Vec::new(),
            selected_connector: 0,
            waveform: waveform::WaveformState::new(),
            frame_count: 0,
        }
    }
}

pub async fn run() -> anyhow::Result<()> {
    let path = socket::default_socket_path();
    let mut stream = UnixStream::connect(&path)?;

    send_subscribe(&mut stream)?;
    let response = read_daemon_message(&mut stream)?;
    validate_subscribe_response(&response)?;

    stream.set_nonblocking(true)?;

    let mut stdout = std::io::stdout();
    terminal::enable_raw_mode()?;
    stdout.execute(EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let mut state = TuiState::new();
    let mut cmd_id: u32 = 1;

    let result = render_loop(&mut terminal, &mut stream, &mut state, &mut cmd_id);

    terminal::disable_raw_mode()?;
    terminal.backend_mut().execute(LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

fn render_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    stream: &mut UnixStream,
    state: &mut TuiState,
    cmd_id: &mut u32,
) -> anyhow::Result<()> {
    loop {
        terminal.draw(|frame| layout::render(state, frame))?;
        state.frame_count = state.frame_count.wrapping_add(1);

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                send_spike(stream, cmd_id)?;
                match key.code {
                    KeyCode::Char('q') => return Ok(()),
                    KeyCode::Char('t') => {
                        if let Some(connector) = state.connectors.get(state.selected_connector) {
                            send_toggle(stream, cmd_id, &connector.name.clone())?;
                        }
                    }
                    KeyCode::Char('r') => {
                        send_reload(stream, cmd_id)?;
                    }
                    KeyCode::Up => {
                        if state.selected_connector > 0 {
                            state.selected_connector -= 1;
                        }
                    }
                    KeyCode::Down => {
                        if state.selected_connector + 1 < state.connectors.len() {
                            state.selected_connector += 1;
                        }
                    }
                    _ => {}
                }
            }
        }

        drain_daemon_messages(stream, state);
    }
}

fn drain_daemon_messages(stream: &mut UnixStream, state: &mut TuiState) {
    loop {
        match read_daemon_message(stream) {
            Ok(buf) => apply_daemon_message(&buf, state),
            Err(_) => break,
        }
    }
}

fn apply_daemon_message(buf: &[u8], state: &mut TuiState) {
    let mut slice = buf;
    let reader = match capnp::serialize::read_message_from_flat_slice(
        &mut slice,
        capnp::message::ReaderOptions::default(),
    ) {
        Ok(r) => r,
        Err(_) => return,
    };

    let daemon_msg = match reader.get_root::<gluebox_capnp::daemon_message::Reader<'_>>() {
        Ok(m) => m,
        Err(_) => return,
    };

    match daemon_msg.which() {
        Ok(gluebox_capnp::daemon_message::Which::State(Ok(snap))) => {
            apply_state_snapshot(snap, state);
        }
        Ok(gluebox_capnp::daemon_message::Which::Activity(Ok(activity))) => {
            apply_activity_event(activity, state);
        }
        Ok(gluebox_capnp::daemon_message::Which::Power(Ok(power_state))) => {
            state.power_state = format_power_state(power_state);
        }
        _ => {}
    }
}

fn apply_state_snapshot(
    snap: gluebox_capnp::state_snapshot::Reader<'_>,
    state: &mut TuiState,
) {
    state.uptime_secs = snap.get_uptime_secs();
    state.potential = snap.get_potential();
    state.threshold = snap.get_threshold();
    state.events_per_min = snap.get_events_per_min();
    state.framerate = snap.get_framerate();
    state.waveform.push(state.potential, state.threshold);

    state.power_state = match snap.get_power_state() {
        Ok(gluebox_capnp::PowerState::Active) => "Active".into(),
        Ok(gluebox_capnp::PowerState::Resting) => "Resting".into(),
        Err(_) => "Unknown".into(),
    };

    state.connectors.clear();
    if let Ok(connectors) = snap.get_connectors() {
        for c in connectors.iter() {
            let name = c.get_name().ok().and_then(|n| n.to_str().ok()).unwrap_or_default().to_owned();
            let status = match c.get_status() {
                Ok(gluebox_capnp::Status::Running) => "running".into(),
                Ok(gluebox_capnp::Status::Stopped) => "stopped".into(),
                Ok(gluebox_capnp::Status::Suspended) => "suspended".into(),
                Ok(gluebox_capnp::Status::Error) => "error".into(),
                Err(_) => "unknown".into(),
            };
            let sparkline = c
                .get_sparkline()
                .map(|s| s.iter().collect())
                .unwrap_or_default();
            let event_count = c.get_event_count();
            let error_message = c
                .get_error_message()
                .ok()
                .and_then(|m| m.to_str().ok())
                .unwrap_or_default()
                .to_owned();

            state.connectors.push(ConnectorInfo {
                name,
                status,
                sparkline,
                event_count,
                error_message,
            });
        }
    }

    if state.selected_connector >= state.connectors.len() && !state.connectors.is_empty() {
        state.selected_connector = state.connectors.len() - 1;
    }
}

fn apply_activity_event(
    activity: gluebox_capnp::activity_event::Reader<'_>,
    state: &mut TuiState,
) {
    let source = activity.get_source().ok().and_then(|s| s.to_str().ok()).unwrap_or_default().to_owned();
    let event_type = activity.get_event_type().ok().and_then(|s| s.to_str().ok()).unwrap_or_default().to_owned();
    let detail = activity.get_detail().ok().and_then(|s| s.to_str().ok()).unwrap_or_default().to_owned();

    state.events.push(EventEntry {
        source,
        event_type,
        detail,
        received_at: Instant::now(),
    });

    const MAX_EVENTS: usize = 100;
    if state.events.len() > MAX_EVENTS {
        state.events.drain(0..state.events.len() - MAX_EVENTS);
    }
}

fn format_power_state(power_state: gluebox_capnp::PowerState) -> String {
    match power_state {
        gluebox_capnp::PowerState::Active => "Active".into(),
        gluebox_capnp::PowerState::Resting => "Resting".into(),
    }
}

fn send_subscribe(stream: &mut UnixStream) -> anyhow::Result<()> {
    let mut message = capnp::message::Builder::new_default();
    {
        let mut cmd = message.init_root::<gluebox_capnp::command::Builder<'_>>();
        cmd.set_id(0);
        cmd.set_subscribe(());
    }
    write_framed_message(stream, &message)
}

fn validate_subscribe_response(buf: &[u8]) -> anyhow::Result<()> {
    let mut slice = buf;
    let reader = capnp::serialize::read_message_from_flat_slice(
        &mut slice,
        capnp::message::ReaderOptions::default(),
    )?;
    let daemon_msg = reader.get_root::<gluebox_capnp::daemon_message::Reader<'_>>()?;
    match daemon_msg.which()? {
        gluebox_capnp::daemon_message::Which::Response(Ok(resp)) => {
            match resp.which()? {
                gluebox_capnp::command_response::Which::Ok(()) => Ok(()),
                gluebox_capnp::command_response::Which::Error(err) => {
                    anyhow::bail!("subscribe failed: {}", err?.to_str()?)
                }
            }
        }
        _ => anyhow::bail!("unexpected response to subscribe"),
    }
}

fn send_spike(stream: &mut UnixStream, cmd_id: &mut u32) -> anyhow::Result<()> {
    *cmd_id += 1;
    let mut message = capnp::message::Builder::new_default();
    {
        let mut cmd = message.init_root::<gluebox_capnp::command::Builder<'_>>();
        cmd.set_id(*cmd_id);
        cmd.set_spike(());
    }
    write_framed_message(stream, &message)
}

fn send_toggle(stream: &mut UnixStream, cmd_id: &mut u32, name: &str) -> anyhow::Result<()> {
    *cmd_id += 1;
    let mut message = capnp::message::Builder::new_default();
    {
        let mut cmd = message.init_root::<gluebox_capnp::command::Builder<'_>>();
        cmd.set_id(*cmd_id);
        cmd.set_toggle(name);
    }
    write_framed_message(stream, &message)
}

fn send_reload(stream: &mut UnixStream, cmd_id: &mut u32) -> anyhow::Result<()> {
    *cmd_id += 1;
    let mut message = capnp::message::Builder::new_default();
    {
        let mut cmd = message.init_root::<gluebox_capnp::command::Builder<'_>>();
        cmd.set_id(*cmd_id);
        cmd.set_reload(());
    }
    write_framed_message(stream, &message)
}

fn write_framed_message(
    stream: &mut UnixStream,
    message: &capnp::message::Builder<capnp::message::HeapAllocator>,
) -> anyhow::Result<()> {
    let mut buf = Vec::new();
    capnp::serialize::write_message(&mut buf, message)?;
    let len = (buf.len() as u32).to_be_bytes();
    stream.write_all(&len)?;
    stream.write_all(&buf)?;
    stream.flush()?;
    Ok(())
}

fn read_daemon_message(stream: &mut UnixStream) -> anyhow::Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf)?;
    let len = u32::from_be_bytes(len_buf) as usize;
    anyhow::ensure!(len <= 1_048_576, "message too large: {len} bytes");
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf)?;
    Ok(buf)
}
