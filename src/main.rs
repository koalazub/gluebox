#![recursion_limit = "512"]
mod config;
mod db;
mod connectors;
mod triggers;
mod webhook;
mod openclaw;

use std::sync::Arc;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("gluebox=info".parse()?))
        .init();

    let cfg = config::Config::load()?;
    let db = db::Db::open(&cfg.db_path)?;

    let anytype = connectors::anytype::AnytypeClient::new(
        &cfg.anytype.api_url,
        &cfg.anytype.api_key,
        &cfg.anytype.space_id,
    );
    anytype.ensure_types().await?;

    let matrix_bot = if let (Some(username), Some(password)) = (&cfg.matrix.bot_username, &cfg.matrix.bot_password) {
        let data_dir = dirs::data_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("gluebox")
            .join("matrix-store");
        std::fs::create_dir_all(&data_dir)?;

        let bot = connectors::matrix::MatrixBot::login(
            &cfg.matrix.homeserver_url,
            username,
            password,
            &cfg.matrix.room_id,
            data_dir,
        ).await?;

        bot.initial_sync().await?;
        Some(Arc::new(bot))
    } else {
        tracing::warn!("matrix bot_username/bot_password not set, E2EE bot disabled");
        None
    };

    let state = Arc::new(AppState {
        cfg,
        db,
        matrix_bot: matrix_bot.clone(),
    });

    if let Some(bot) = matrix_bot {
        let openclaw_state = state.clone();
        tokio::spawn(openclaw::start_openclaw(openclaw_state, bot));
    }

    let app = webhook::router(state.clone());

    let addr = state.cfg.listen_addr.clone();
    tracing::info!(%addr, "gluebox starting");

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

pub struct AppState {
    pub cfg: config::Config,
    pub db: db::Db,
    pub matrix_bot: Option<Arc<connectors::matrix::MatrixBot>>,
}
