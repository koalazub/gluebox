#![recursion_limit = "1024"]
mod config;
mod db;
mod connectors;
mod triggers;
mod webhook;
mod daemon;
mod socket;
mod gluebox_capnp;
mod tui;
mod mcp;
mod gateway_tools;
mod gateway;
mod gateway_socket;

use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use tracing_subscriber::EnvFilter;
use clap::Parser;

#[derive(Parser)]
#[command(name = "gluebox", about = "Runtime-configurable service daemon")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Enable debug logging for the gateway (stdout/stderr sizes, etc.)
    #[arg(long, global = true)]
    debug: bool,
}

#[derive(clap::Subcommand)]
enum Commands {
    Tui,
    Status,
    Reload,
    Toggle { connector: String },
    Mcp {
        #[arg(long, default_value = "http://127.0.0.1:8990")]
        daemon_url: String,
        #[arg(long, env = "GLUEBOX_NOTIFY_SECRET", default_value = "")]
        auth_token: String,
    },
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
            let registry = Arc::new(gluebox_core::ConnectorRegistry::new());

            if let Some(ref linear_cfg) = cfg.linear {
                let connector = Arc::new(connectors::linear::LinearConnector::new(linear_cfg.clone()));
                registry.register("linear".into(), connector).await?;
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

            if !cfg.affine.is_empty() {
                let connector = Arc::new(connectors::affine::AffineConnector::new(cfg.affine.clone()));
                registry.register("affine".into(), connector).await?;
            }

            let heartbeat = triggers::posting_heartbeat::PostingHeartbeat::new();

            if let Some(social_cfg) = cfg.stonkwatch_social.as_ref() {
                heartbeat
                    .set_expected_platforms(
                        triggers::posting_heartbeat::expected_platforms_from_config(social_cfg),
                    )
                    .await;
            }

            if let Some(ref social_cfg) = cfg.stonkwatch_social {
                let connector = Arc::new(
                    connectors::stonkwatch_social::StonkwatchSocialConnector::new(
                        social_cfg.clone(),
                        heartbeat.clone(),
                    ),
                );
                registry.register("stonkwatch_social".into(), connector).await?;
            }

            let power_config = cfg.power.clone().unwrap_or_default();
            let power = Arc::new(gluebox_core::PowerManager::new(power_config)?);

            let (events_tx, _) = broadcast::channel::<socket::ActivityEventData>(256);

            let error_rollup = triggers::error_rollup::ErrorRollup::new();

            triggers::posting_heartbeat::spawn_watchdog(heartbeat.clone(), error_rollup.clone());

            let state = Arc::new(AppState {
                registry: registry.clone(),
                db,
                config: Arc::new(RwLock::new(cfg)),
                power: power.clone(),
                started_at: std::time::Instant::now(),
                events_tx: events_tx.clone(),
                error_rollup: error_rollup.clone(),
                heartbeat: heartbeat.clone(),
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
                        gluebox_core::PowerState::Active => {
                            tracing::info!("power: transitioning to Active");
                            watch_registry.resume_all().await;
                        }
                        gluebox_core::PowerState::Resting => {
                            tracing::info!("power: transitioning to Resting");
                            watch_registry.suspend_all().await;
                        }
                    }
                }
            });

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

            {
                let cfg = state.config.read().await;
                let base_path = cfg.socket_path.clone().unwrap_or_else(socket::default_socket_path);
                drop(cfg);
                let gateway_path = format!("{base_path}.gateway");
                let debug_mode = cli.debug;
                tokio::spawn(async move {
                    if let Err(e) = gateway_socket::run_gateway(&gateway_path, debug_mode).await {
                        tracing::error!("gateway socket error: {e}");
                    }
                });
            }

            {
                let state_clone = state.clone();
                tokio::spawn(async move {
                    let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
                    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                    loop {
                        interval.tick().await;
                        if let Err(e) = crate::triggers::friday_digest::run_if_scheduled(&state_clone).await {
                            tracing::error!(error = %e, "friday_digest tick failed");
                        }
                    }
                });
            }

            {
                let cfg = state.config.read().await;
                let rollup_room = cfg.matrix.as_ref().and_then(|m| m.error_rollup_room_id.clone());
                drop(cfg);
                if let Some(room_id) = rollup_room {
                    let conn = state.registry.get_dyn("matrix").await;
                    if let Some(conn) = conn {
                        use crate::connectors::matrix::MatrixConnector;
                        if let Some(matrix_connector) = conn.as_any().downcast_ref::<MatrixConnector>() {
                            if let Ok(bot) = matrix_connector.bot().await {
                                triggers::error_rollup::spawn_flush_loop(error_rollup, bot, room_id);
                            } else {
                                tracing::warn!("error-rollup: matrix connector not running; error rollup disabled");
                            }
                        } else {
                            tracing::warn!("error-rollup: matrix connector not found/downcastable; error rollup disabled");
                        }
                    } else {
                        tracing::warn!("error-rollup: matrix connector not found/downcastable; error rollup disabled");
                    }
                }
            }

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
            tui::run().await?;
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
        Some(Commands::Mcp { daemon_url, auth_token }) => {
            mcp::run(daemon_url, auth_token).await?;
        }
    }

    Ok(())
}

pub struct AppState {
    pub registry: Arc<gluebox_core::ConnectorRegistry>,
    pub db: Arc<db::Db>,
    pub config: Arc<RwLock<config::Config>>,
    pub power: Arc<gluebox_core::PowerManager>,
    pub started_at: std::time::Instant,
    pub events_tx: broadcast::Sender<socket::ActivityEventData>,
    pub error_rollup: Arc<triggers::error_rollup::ErrorRollup>,
    pub heartbeat: Arc<triggers::posting_heartbeat::PostingHeartbeat>,
}
