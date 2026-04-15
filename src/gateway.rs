use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::process::Command;

use crate::gateway_tools::{is_command_allowed, tool_registry};
use crate::gluebox_capnp;

pub async fn handle_gateway_client(mut stream: UnixStream, debug_mode: bool) {
    if let Err(e) = gateway_loop(&mut stream, debug_mode).await {
        tracing::debug!("gateway connection closed: {e}");
    }
}

async fn gateway_loop(stream: &mut UnixStream, debug_mode: bool) -> anyhow::Result<()> {
    loop {
        let buf = read_framed_message(stream).await?;
        let response_bytes = dispatch_gateway_command(&buf, debug_mode).await?;
        write_framed_message(stream, &response_bytes).await?;
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

async fn dispatch_gateway_command(buf: &[u8], debug_mode: bool) -> anyhow::Result<Vec<u8>> {
    let mut slice = buf;
    let message_reader = capnp::serialize::read_message_from_flat_slice(
        &mut slice,
        capnp::message::ReaderOptions::default(),
    )?;
    let cmd = message_reader.get_root::<gluebox_capnp::gateway_command::Reader<'_>>()?;
    let cmd_id = cmd.get_id();

    match cmd.which()? {
        gluebox_capnp::gateway_command::Which::GetCapabilities(()) => {
            tracing::info!(id = cmd_id, "gateway: getCapabilities");
            build_capabilities_response(cmd_id)
        }
        gluebox_capnp::gateway_command::Which::Run(cmd_text_result) => {
            let command_str = cmd_text_result?.to_string()?;
            if !is_command_allowed(&command_str) {
                tracing::warn!(id = cmd_id, command = %command_str, "gateway: rejected disallowed command");
                return build_error_response(cmd_id, &format!("command not allowed: {command_str}"));
            }
            tracing::info!(id = cmd_id, command = %command_str, "gateway: run");
            let result = run_command(&command_str).await?;
            if debug_mode {
                tracing::debug!(
                    id = cmd_id,
                    stdout_bytes = result.stdout.len(),
                    stderr_bytes = result.stderr.len(),
                    "gateway: run complete"
                );
            }
            build_run_result_response(cmd_id, &result.stdout, &result.stderr, result.exit_code)
        }
    }
}

struct RunOutput {
    stdout: String,
    stderr: String,
    exit_code: i32,
}

async fn run_command(command_str: &str) -> anyhow::Result<RunOutput> {
    let output = Command::new("/bin/sh")
        .arg("-c")
        .arg(command_str)
        .output()
        .await?;

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let exit_code = output.status.code().unwrap_or(-1);

    Ok(RunOutput { stdout, stderr, exit_code })
}

fn build_capabilities_response(id: u32) -> anyhow::Result<Vec<u8>> {
    let tools = tool_registry();
    let mut outer = capnp::message::Builder::new_default();
    {
        let mut resp = outer.init_root::<gluebox_capnp::gateway_response::Builder<'_>>();
        resp.set_id(id);
        let mut caps = resp.init_capabilities();
        caps.set_version("0.1.0");
        let mut tool_list = caps.init_tools(tools.len() as u32);
        for (i, entry) in tools.iter().enumerate() {
            let mut t = tool_list.reborrow().get(i as u32);
            t.set_name(entry.name);
            t.set_description(entry.description);
            t.set_example(entry.example);
        }
    }
    serialize_message(&outer)
}

fn build_run_result_response(
    id: u32,
    stdout: &str,
    stderr: &str,
    exit_code: i32,
) -> anyhow::Result<Vec<u8>> {
    let mut outer = capnp::message::Builder::new_default();
    {
        let mut resp = outer.init_root::<gluebox_capnp::gateway_response::Builder<'_>>();
        resp.set_id(id);
        let mut result = resp.init_result();
        result.set_stdout(stdout);
        result.set_stderr(stderr);
        result.set_exit_code(exit_code);
    }
    serialize_message(&outer)
}

fn build_error_response(id: u32, message: &str) -> anyhow::Result<Vec<u8>> {
    let mut outer = capnp::message::Builder::new_default();
    {
        let mut resp = outer.init_root::<gluebox_capnp::gateway_response::Builder<'_>>();
        resp.set_id(id);
        resp.set_error(message);
    }
    serialize_message(&outer)
}

fn serialize_message(
    message: &capnp::message::Builder<capnp::message::HeapAllocator>,
) -> anyhow::Result<Vec<u8>> {
    let mut buf = Vec::new();
    capnp::serialize::write_message(&mut buf, message)?;
    Ok(buf)
}
