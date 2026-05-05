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

    if old_cfg.affine != new_cfg.affine {
        if new_cfg.affine.is_empty() {
            state.registry.deregister("affine").await?;
            changes.push("affine: removed".into());
        } else if old_cfg.affine.is_empty() {
            let connector = Arc::new(connectors::affine::AffineConnector::new(new_cfg.affine.clone()));
            state.registry.register("affine".into(), connector).await?;
            changes.push("affine: added".into());
        } else {
            if let Some(conn) = state.registry.get_dyn("affine").await {
                let toml_val = toml::Value::try_from(new_cfg.affine.clone())?;
                let reconfigured = conn.reconfigure(&toml_val).await?;
                if !reconfigured {
                    state.registry.deregister("affine").await?;
                    let connector = Arc::new(connectors::affine::AffineConnector::new(new_cfg.affine.clone()));
                    state.registry.register("affine".into(), connector).await?;
                }
            }
            changes.push("affine: reconfigured".into());
        }
    }

    match (&old_cfg.stonkwatch_social, &new_cfg.stonkwatch_social) {
        (None, Some(new)) => {
            state
                .heartbeat
                .set_expected_platforms(
                    crate::triggers::posting_heartbeat::expected_platforms_from_config(new),
                )
                .await;
            let connector = Arc::new(
                connectors::stonkwatch_social::StonkwatchSocialConnector::new(
                    new.clone(),
                    state.heartbeat.clone(),
                ),
            );
            state.registry.register("stonkwatch_social".into(), connector).await?;
            changes.push("stonkwatch_social: added".into());
        }
        (Some(_), None) => {
            state.heartbeat.set_expected_platforms(Vec::new()).await;
            state.registry.deregister("stonkwatch_social").await?;
            changes.push("stonkwatch_social: removed".into());
        }
        (Some(old), Some(new)) if old != new => {
            state
                .heartbeat
                .set_expected_platforms(
                    crate::triggers::posting_heartbeat::expected_platforms_from_config(new),
                )
                .await;
            if let Some(conn) = state.registry.get_dyn("stonkwatch_social").await {
                let toml_val = toml::Value::try_from(new.clone())?;
                let reconfigured = conn.reconfigure(&toml_val).await?;
                if !reconfigured {
                    state.registry.deregister("stonkwatch_social").await?;
                    let connector = Arc::new(
                        connectors::stonkwatch_social::StonkwatchSocialConnector::new(
                            new.clone(),
                            state.heartbeat.clone(),
                        ),
                    );
                    state.registry.register("stonkwatch_social".into(), connector).await?;
                }
            }
            changes.push("stonkwatch_social: reconfigured".into());
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
