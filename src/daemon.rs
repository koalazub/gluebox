use std::sync::Arc;
use crate::AppState;
use crate::config::Config;
use crate::connectors;

pub async fn reload(state: &Arc<AppState>) -> anyhow::Result<String> {
    let new_cfg = Config::load()?;
    let old_cfg = state.config.read().await.clone();

    let mut changes: Vec<String> = Vec::new();

    match (&old_cfg.linear, &new_cfg.linear) {
        (None, Some(new)) => {
            let connector = Arc::new(connectors::linear::LinearConnector::new(new.clone()));
            state.registry.register("linear".into(), connector).await?;
            changes.push("linear: added".into());
        }
        (Some(_), None) => {
            state.registry.deregister("linear").await?;
            changes.push("linear: removed".into());
        }
        (Some(old), Some(new)) if old != new => {
            if let Some(conn) = state.registry.get_dyn("linear").await {
                let toml_val = toml::Value::try_from(new.clone())?;
                let reconfigured = conn.reconfigure(&toml_val).await?;
                if !reconfigured {
                    state.registry.deregister("linear").await?;
                    let connector = Arc::new(connectors::linear::LinearConnector::new(new.clone()));
                    state.registry.register("linear".into(), connector).await?;
                }
            }
            changes.push("linear: reconfigured".into());
        }
        _ => {}
    }

    match (&old_cfg.anytype, &new_cfg.anytype) {
        (None, Some(new)) => {
            let connector = Arc::new(connectors::anytype::AnytypeConnector::new(new.clone()));
            state.registry.register("anytype".into(), connector).await?;
            changes.push("anytype: added".into());
        }
        (Some(_), None) => {
            state.registry.deregister("anytype").await?;
            changes.push("anytype: removed".into());
        }
        (Some(old), Some(new)) if old != new => {
            if let Some(conn) = state.registry.get_dyn("anytype").await {
                let toml_val = toml::Value::try_from(new.clone())?;
                let reconfigured = conn.reconfigure(&toml_val).await?;
                if !reconfigured {
                    state.registry.deregister("anytype").await?;
                    let connector = Arc::new(connectors::anytype::AnytypeConnector::new(new.clone()));
                    state.registry.register("anytype".into(), connector).await?;
                }
            }
            changes.push("anytype: reconfigured".into());
        }
        _ => {}
    }

    match (&old_cfg.matrix, &new_cfg.matrix) {
        (None, Some(new)) => {
            let connector = Arc::new(connectors::matrix::MatrixConnector::new(new.clone()));
            state.registry.register("matrix".into(), connector).await?;
            changes.push("matrix: added".into());
        }
        (Some(_), None) => {
            state.registry.deregister("matrix").await?;
            changes.push("matrix: removed".into());
        }
        (Some(old), Some(new)) if old != new => {
            if let Some(conn) = state.registry.get_dyn("matrix").await {
                let toml_val = toml::Value::try_from(new.clone())?;
                let reconfigured = conn.reconfigure(&toml_val).await?;
                if !reconfigured {
                    state.registry.deregister("matrix").await?;
                    let connector = Arc::new(connectors::matrix::MatrixConnector::new(new.clone()));
                    state.registry.register("matrix".into(), connector).await?;
                }
            }
            changes.push("matrix: reconfigured".into());
        }
        _ => {}
    }

    match (&old_cfg.documenso, &new_cfg.documenso) {
        (None, Some(_new)) => {
            let connector = Arc::new(connectors::documenso::DocumensoConnector::new());
            state.registry.register("documenso".into(), connector).await?;
            changes.push("documenso: added".into());
        }
        (Some(_), None) => {
            state.registry.deregister("documenso").await?;
            changes.push("documenso: removed".into());
        }
        (Some(old), Some(new)) if old != new => {
            state.registry.deregister("documenso").await?;
            let connector = Arc::new(connectors::documenso::DocumensoConnector::new());
            state.registry.register("documenso".into(), connector).await?;
            changes.push("documenso: reconfigured".into());
        }
        _ => {}
    }

    match (&old_cfg.github, &new_cfg.github) {
        (None, Some(new)) => {
            let connector = Arc::new(connectors::github::GithubConnector::new(new.clone()));
            state.registry.register("github".into(), connector).await?;
            changes.push("github: added".into());
        }
        (Some(_), None) => {
            state.registry.deregister("github").await?;
            changes.push("github: removed".into());
        }
        (Some(old), Some(new)) if old != new => {
            if let Some(conn) = state.registry.get_dyn("github").await {
                let toml_val = toml::Value::try_from(new.clone())?;
                let reconfigured = conn.reconfigure(&toml_val).await?;
                if !reconfigured {
                    state.registry.deregister("github").await?;
                    let connector = Arc::new(connectors::github::GithubConnector::new(new.clone()));
                    state.registry.register("github".into(), connector).await?;
                }
            }
            changes.push("github: reconfigured".into());
        }
        _ => {}
    }

    match (&old_cfg.opencode, &new_cfg.opencode) {
        (None, Some(new)) => {
            let connector = Arc::new(connectors::opencode::OpenCodeConnector::new(new.clone()));
            state.registry.register("opencode".into(), connector).await?;
            changes.push("opencode: added".into());
        }
        (Some(_), None) => {
            state.registry.deregister("opencode").await?;
            changes.push("opencode: removed".into());
        }
        (Some(old), Some(new)) if old != new => {
            if let Some(conn) = state.registry.get_dyn("opencode").await {
                let toml_val = toml::Value::try_from(new.clone())?;
                let reconfigured = conn.reconfigure(&toml_val).await?;
                if !reconfigured {
                    state.registry.deregister("opencode").await?;
                    let connector = Arc::new(connectors::opencode::OpenCodeConnector::new(new.clone()));
                    state.registry.register("opencode".into(), connector).await?;
                }
            }
            changes.push("opencode: reconfigured".into());
        }
        _ => {}
    }

    match (&old_cfg.affine, &new_cfg.affine) {
        (None, Some(new)) => {
            let connector = Arc::new(connectors::affine::AffineConnector::new(new.clone()));
            state.registry.register("affine".into(), connector).await?;
            changes.push("affine: added".into());
        }
        (Some(_), None) => {
            state.registry.deregister("affine").await?;
            changes.push("affine: removed".into());
        }
        (Some(old), Some(new)) if old != new => {
            if let Some(conn) = state.registry.get_dyn("affine").await {
                let toml_val = toml::Value::try_from(new.clone())?;
                let reconfigured = conn.reconfigure(&toml_val).await?;
                if !reconfigured {
                    state.registry.deregister("affine").await?;
                    let connector = Arc::new(connectors::affine::AffineConnector::new(new.clone()));
                    state.registry.register("affine".into(), connector).await?;
                }
            }
            changes.push("affine: reconfigured".into());
        }
        _ => {}
    }

    match (&old_cfg.watcher, &new_cfg.watcher) {
        (None, Some(new)) => {
            let connector = Arc::new(connectors::session_watcher::SessionWatcherConnector::new(
                new.clone(),
                Box::new(|session_id| {
                    tracing::info!(session_id, "watcher detected session ready for import");
                }),
            ));
            state.registry.register("watcher".into(), connector).await?;
            changes.push("watcher: added".into());
        }
        (Some(_), None) => {
            state.registry.deregister("watcher").await?;
            changes.push("watcher: removed".into());
        }
        (Some(old), Some(new)) if old != new => {
            state.registry.deregister("watcher").await?;
            let connector = Arc::new(connectors::session_watcher::SessionWatcherConnector::new(
                new.clone(),
                Box::new(|session_id| {
                    tracing::info!(session_id, "watcher detected session ready for import");
                }),
            ));
            state.registry.register("watcher".into(), connector).await?;
            changes.push("watcher: reconfigured".into());
        }
        _ => {}
    }

    if old_cfg.power != new_cfg.power {
        let new_power = new_cfg.power.clone().unwrap_or_default();
        state.power.reconfigure(new_power)?;
        changes.push("power: updated".into());
    }

    *state.config.write().await = new_cfg;

    if changes.is_empty() {
        Ok("reload complete: no changes".into())
    } else {
        Ok(format!("reload complete: {}", changes.join(", ")))
    }
}
