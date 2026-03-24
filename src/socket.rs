use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::sync::broadcast;

use crate::connector::ConnectorStatus;
use crate::gluebox_capnp;
use crate::power::PowerState;
use crate::AppState;

#[derive(Clone, Debug)]
pub struct ActivityEventData {
    pub timestamp_ms: u64,
    pub source: String,
    pub event_type: String,
    pub detail: String,
}

pub fn default_socket_path() -> String {
    std::env::var("XDG_RUNTIME_DIR")
        .map(|dir| format!("{dir}/gluebox.sock"))
        .unwrap_or_else(|_| "/tmp/gluebox.sock".into())
}

pub async fn run(
    state: Arc<AppState>,
    events_tx: broadcast::Sender<ActivityEventData>,
) -> anyhow::Result<()> {
    let cfg = state.config.read().await;
    let path = cfg.socket_path.clone().unwrap_or_else(default_socket_path);
    drop(cfg);

    if std::path::Path::new(&path).exists() {
        let _ = std::fs::remove_file(&path);
    }

    let listener = tokio::net::UnixListener::bind(&path)?;
    tracing::info!(path, "socket server listening");

    loop {
        let (stream, _) = listener.accept().await?;
        let state = state.clone();
        let events_rx = events_tx.subscribe();
        tokio::spawn(handle_connection(state, stream, events_rx));
    }
}

async fn handle_connection(
    state: Arc<AppState>,
    mut stream: UnixStream,
    events_rx: broadcast::Receiver<ActivityEventData>,
) {
    if let Err(e) = connection_loop(&state, &mut stream, events_rx).await {
        tracing::debug!("socket connection closed: {e}");
    }
}

async fn connection_loop(
    state: &Arc<AppState>,
    stream: &mut UnixStream,
    mut events_rx: broadcast::Receiver<ActivityEventData>,
) -> anyhow::Result<()> {
    let mut subscribed = false;

    loop {
        if subscribed {
            let tick_duration = snapshot_interval(state);
            tokio::select! {
                biased;
                result = read_framed_message(stream) => {
                    let buf = result?;
                    let response_bytes = dispatch_command(state, &buf, &mut subscribed).await?;
                    write_framed_message(stream, &response_bytes).await?;
                }
                result = events_rx.recv() => {
                    if let Ok(event) = result {
                        let msg = build_activity_message(&event)?;
                        write_framed_message(stream, &msg).await?;
                    }
                }
                _ = tokio::time::sleep(tick_duration) => {
                    let msg = build_state_message(state).await?;
                    write_framed_message(stream, &msg).await?;
                }
            }
        } else {
            let buf = read_framed_message(stream).await?;
            let response_bytes = dispatch_command(state, &buf, &mut subscribed).await?;
            write_framed_message(stream, &response_bytes).await?;
        }
    }
}

async fn read_framed_message(stream: &mut UnixStream) -> anyhow::Result<Vec<u8>> {
    let len = stream.read_u32().await?;
    anyhow::ensure!(len <= 1_048_576, "message too large: {len} bytes");
    let mut buf = vec![0u8; len as usize];
    stream.read_exact(&mut buf).await?;
    Ok(buf)
}

async fn write_framed_message(stream: &mut UnixStream, data: &[u8]) -> anyhow::Result<()> {
    stream.write_u32(data.len() as u32).await?;
    stream.write_all(data).await?;
    stream.flush().await?;
    Ok(())
}

fn snapshot_interval(state: &AppState) -> Duration {
    match state.power.state() {
        PowerState::Active => Duration::from_millis(100),
        PowerState::Resting => Duration::from_millis(500),
    }
}

async fn dispatch_command(
    state: &Arc<AppState>,
    buf: &[u8],
    subscribed: &mut bool,
) -> anyhow::Result<Vec<u8>> {
    let mut slice = buf;
    let message_reader = capnp::serialize::read_message_from_flat_slice(
        &mut slice,
        capnp::message::ReaderOptions::default(),
    )?;
    let cmd = message_reader.get_root::<gluebox_capnp::command::Reader<'_>>()?;
    let cmd_id = cmd.get_id();

    match cmd.which()? {
        gluebox_capnp::command::Which::Status(()) => {
            let snapshot_msg = build_state_message(state).await?;
            Ok(snapshot_msg)
        }
        gluebox_capnp::command::Which::Toggle(name_result) => {
            let name = name_result?.to_string()?;
            let result = state.registry.toggle(&name).await;
            build_response_message(cmd_id, result.map(|_| ()))
        }
        gluebox_capnp::command::Which::Reload(()) => {
            let result = crate::daemon::reload(state).await;
            build_response_message(cmd_id, result.map(|_| ()))
        }
        gluebox_capnp::command::Which::Spike(()) => {
            state.power.spike();
            build_response_message(cmd_id, Ok(()))
        }
        gluebox_capnp::command::Which::Subscribe(()) => {
            *subscribed = true;
            build_response_message(cmd_id, Ok(()))
        }
    }
}

fn build_response_message(id: u32, result: anyhow::Result<()>) -> anyhow::Result<Vec<u8>> {
    let mut outer = capnp::message::Builder::new_default();
    {
        let daemon_msg = outer.init_root::<gluebox_capnp::daemon_message::Builder<'_>>();
        let mut resp = daemon_msg.init_response();
        resp.set_id(id);
        match result {
            Ok(()) => resp.set_ok(()),
            Err(e) => resp.set_error(&format!("{e}")),
        }
    }
    serialize_message(&outer)
}

async fn build_state_message(state: &AppState) -> anyhow::Result<Vec<u8>> {
    let connector_list = state.registry.list().await;
    let uptime_secs = state.started_at.elapsed().as_secs();
    let potential = state.power.potential();
    let threshold = state.power.threshold();
    let power_state = state.power.state();

    let mut outer = capnp::message::Builder::new_default();
    {
        let daemon_msg = outer.init_root::<gluebox_capnp::daemon_message::Builder<'_>>();
        let mut snap = daemon_msg.init_state();

        snap.set_uptime_secs(uptime_secs);
        snap.set_potential(potential);
        snap.set_threshold(threshold);

        let capnp_power_state = match power_state {
            PowerState::Active => gluebox_capnp::PowerState::Active,
            PowerState::Resting => gluebox_capnp::PowerState::Resting,
        };
        snap.set_power_state(capnp_power_state);

        snap.set_events_per_min(0.0);

        let framerate = match power_state {
            PowerState::Active => 10u8,
            PowerState::Resting => 2u8,
        };
        snap.set_framerate(framerate);

        let mut connectors = snap.init_connectors(connector_list.len() as u32);
        for (i, (name, status)) in connector_list.iter().enumerate() {
            let mut entry = connectors.reborrow().get(i as u32);
            entry.set_name(name);
            let capnp_status = match status {
                ConnectorStatus::Running => gluebox_capnp::Status::Running,
                ConnectorStatus::Stopped => gluebox_capnp::Status::Stopped,
                ConnectorStatus::Suspended => gluebox_capnp::Status::Suspended,
                ConnectorStatus::Error(_) => gluebox_capnp::Status::Error,
            };
            entry.set_status(capnp_status);
            if let ConnectorStatus::Error(msg) = status {
                entry.reborrow().set_error_message(msg);
            }
            entry.reborrow().set_event_count(0);
            entry.init_sparkline(0);
        }
    }
    serialize_message(&outer)
}

fn build_activity_message(event: &ActivityEventData) -> anyhow::Result<Vec<u8>> {
    let mut outer = capnp::message::Builder::new_default();
    {
        let daemon_msg = outer.init_root::<gluebox_capnp::daemon_message::Builder<'_>>();
        let mut activity = daemon_msg.init_activity();
        activity.set_timestamp_ms(event.timestamp_ms);
        activity.set_source(&event.source);
        activity.set_event_type(&event.event_type);
        activity.set_detail(&event.detail);
    }
    serialize_message(&outer)
}

fn serialize_message(message: &capnp::message::Builder<capnp::message::HeapAllocator>) -> anyhow::Result<Vec<u8>> {
    let mut buf = Vec::new();
    capnp::serialize::write_message(&mut buf, message)?;
    Ok(buf)
}

pub async fn cleanup_socket(state: &AppState) {
    let cfg = state.config.read().await;
    let path = cfg.socket_path.clone().unwrap_or_else(default_socket_path);
    drop(cfg);
    let _ = std::fs::remove_file(&path);
}
