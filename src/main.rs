#![recursion_limit = "512"]
mod config;
mod power;
mod db;
mod connectors;
mod triggers;
mod webhook;
mod openclaw;
mod connector;
mod registry;
mod daemon;
mod socket;
mod gluebox_capnp;

use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use tracing_subscriber::EnvFilter;
use clap::Parser;

#[derive(Parser)]
#[command(name = "gluebox", about = "Runtime-configurable service daemon")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(clap::Subcommand)]
enum Commands {
    Tui,
    Status,
    Reload,
    Toggle { connector: String },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("gluebox=info".parse()?))
        .init();

    let cli = Cli::parse();

    match cli.command {
        None => {
            let cfg = config::Config::load()?;
            let db = Arc::new(db::Db::open(&cfg.turso).await?);

            let listen_addr = cfg.listen_addr.clone();
            let registry = Arc::new(registry::ConnectorRegistry::new());

            if let Some(ref linear_cfg) = cfg.linear {
                let connector = Arc::new(connectors::linear::LinearConnector::new(linear_cfg.clone()));
                registry.register("linear".into(), connector).await?;
            }

            if let Some(ref anytype_cfg) = cfg.anytype {
                let connector = Arc::new(connectors::anytype::AnytypeConnector::new(anytype_cfg.clone()));
                registry.register("anytype".into(), connector).await?;
            }

            if let Some(ref matrix_cfg) = cfg.matrix {
                let connector = Arc::new(connectors::matrix::MatrixConnector::new(matrix_cfg.clone()));
                registry.register("matrix".into(), connector).await?;
            }

            if let Some(ref _documenso_cfg) = cfg.documenso {
                let connector = Arc::new(connectors::documenso::DocumensoConnector::new());
                registry.register("documenso".into(), connector).await?;
            }

            if let Some(ref github_cfg) = cfg.github {
                let connector = Arc::new(connectors::github::GithubConnector::new(github_cfg.clone()));
                registry.register("github".into(), connector).await?;
            }

            if let Some(ref opencode_cfg) = cfg.opencode {
                let connector = Arc::new(connectors::opencode::OpenCodeConnector::new(opencode_cfg.clone()));
                registry.register("opencode".into(), connector).await?;
            }

            let power_config = cfg.power.clone().unwrap_or_default();
            let power = Arc::new(power::PowerManager::new(power_config)?);

            let (events_tx, _) = broadcast::channel::<socket::ActivityEventData>(256);

            let state = Arc::new(AppState {
                registry: registry.clone(),
                db,
                config: Arc::new(RwLock::new(cfg)),
                power: power.clone(),
                started_at: std::time::Instant::now(),
                events_tx: events_tx.clone(),
            });

            let tick_power = state.power.clone();
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(tick_power.tick_interval()).await;
                    tick_power.tick();
                }
            });

            let watch_power = state.power.clone();
            let watch_registry = state.registry.clone();
            tokio::spawn(async move {
                let mut rx = watch_power.subscribe();
                while rx.changed().await.is_ok() {
                    let current = *rx.borrow();
                    match current {
                        power::PowerState::Active => {
                            tracing::info!("power: transitioning to Active");
                            watch_registry.resume_all().await;
                        }
                        power::PowerState::Resting => {
                            tracing::info!("power: transitioning to Resting");
                            watch_registry.suspend_all().await;
                        }
                    }
                }
            });

            if let Some(matrix_conn) = registry.get_dyn("matrix").await {
                let matrix_connector = matrix_conn
                    .as_any()
                    .downcast_ref::<connectors::matrix::MatrixConnector>()
                    .expect("registry 'matrix' entry is not MatrixConnector");
                let bot = matrix_connector.bot().await?;
                let openclaw_state = state.clone();
                tokio::spawn(openclaw::start_openclaw(openclaw_state, bot));
            }

            let sighup_state = state.clone();
            tokio::spawn(async move {
                let mut sighup = tokio::signal::unix::signal(
                    tokio::signal::unix::SignalKind::hangup(),
                )
                .expect("failed to register SIGHUP handler");
                loop {
                    sighup.recv().await;
                    tracing::info!("SIGHUP received, reloading config");
                    match daemon::reload(&sighup_state).await {
                        Ok(msg) => tracing::info!("reload: {msg}"),
                        Err(e) => tracing::error!("reload failed: {e}"),
                    }
                }
            });

            let socket_state = state.clone();
            let socket_events_tx = events_tx.clone();
            tokio::spawn(async move {
                if let Err(e) = socket::run(socket_state, socket_events_tx).await {
                    tracing::error!("socket server error: {e}");
                }
            });

            let app = webhook::router(state.clone());

            tracing::info!(%listen_addr, "gluebox starting");

            let shutdown_state = state.clone();
            let listener = tokio::net::TcpListener::bind(&listen_addr).await?;
            axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    tokio::signal::ctrl_c().await.ok();
                    tracing::info!("shutdown signal received");
                    shutdown_state.registry.stop_all().await;
                    socket::cleanup_socket(&shutdown_state).await;
                })
                .await?;
        }
        Some(Commands::Tui) => {
            eprintln!("TUI not yet implemented");
        }
        Some(Commands::Status) => {
            let resp = reqwest::get("http://127.0.0.1:8990/admin/status").await?;
            println!("{}", resp.text().await?);
        }
        Some(Commands::Reload) => {
            let client = reqwest::Client::new();
            let resp = client.post("http://127.0.0.1:8990/admin/reload").send().await?;
            println!("{}", resp.text().await?);
        }
        Some(Commands::Toggle { connector }) => {
            let client = reqwest::Client::new();
            let resp = client.post(format!("http://127.0.0.1:8990/admin/connectors/{connector}/toggle")).send().await?;
            println!("{}", resp.text().await?);
        }
    }

    Ok(())
}

pub struct AppState {
    pub registry: Arc<registry::ConnectorRegistry>,
    pub db: Arc<db::Db>,
    pub config: Arc<RwLock<config::Config>>,
    pub power: Arc<power::PowerManager>,
    pub started_at: std::time::Instant,
    pub events_tx: broadcast::Sender<socket::ActivityEventData>,
}
