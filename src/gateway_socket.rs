pub async fn run_gateway(socket_path: &str, debug_mode: bool) -> anyhow::Result<()> {
    if std::path::Path::new(socket_path).exists() {
        let _ = std::fs::remove_file(socket_path);
    }

    let listener = tokio::net::UnixListener::bind(socket_path)?;
    tracing::info!(path = socket_path, "gateway socket listening");

    loop {
        let (stream, _) = listener.accept().await?;
        let debug = debug_mode;
        tokio::spawn(crate::gateway::handle_gateway_client(stream, debug));
    }
}
