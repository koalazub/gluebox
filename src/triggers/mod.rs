mod linear_to_anytype;
pub(crate) mod to_matrix;
mod documenso_handlers;
mod github_to_linear;
mod linear_to_github;
pub mod session_import;
pub mod study_plan;

pub use linear_to_anytype::{linear_issue_created, linear_issue_updated};
pub use documenso_handlers::{documenso_completed, documenso_rejected};
pub use github_to_linear::github_issue_opened;
pub use linear_to_github::linear_issue_github_sync;

use std::sync::Arc;
use crate::AppState;
use crate::connectors::linear::{LinearClient, LinearConnector};
use crate::connectors::anytype::{AnytypeClient, AnytypeConnector};
use crate::connectors::github::{GithubClient, GithubConnector};
use crate::connectors::opencode::{OpenCodeClient, OpenCodeConnector};
use crate::connectors::affine::{AffineClient, AffineConnector};

pub(crate) async fn linear_from_registry(state: &Arc<AppState>) -> anyhow::Result<LinearClient> {
    let conn = state.registry.get_dyn("linear").await
        .ok_or_else(|| anyhow::anyhow!("linear connector not available"))?;
    conn.as_any()
        .downcast_ref::<LinearConnector>()
        .ok_or_else(|| anyhow::anyhow!("linear connector type mismatch"))?
        .client()
        .await
}

pub(crate) async fn anytype_from_registry(state: &Arc<AppState>) -> anyhow::Result<AnytypeClient> {
    let conn = state.registry.get_dyn("anytype").await
        .ok_or_else(|| anyhow::anyhow!("anytype connector not available"))?;
    conn.as_any()
        .downcast_ref::<AnytypeConnector>()
        .ok_or_else(|| anyhow::anyhow!("anytype connector type mismatch"))?
        .client()
        .await
}

pub(crate) async fn github_from_registry(state: &Arc<AppState>) -> anyhow::Result<GithubClient> {
    let conn = state.registry.get_dyn("github").await
        .ok_or_else(|| anyhow::anyhow!("github connector not available"))?;
    conn.as_any()
        .downcast_ref::<GithubConnector>()
        .ok_or_else(|| anyhow::anyhow!("github connector type mismatch"))?
        .client()
        .await
}

pub(crate) async fn opencode_from_registry(state: &Arc<AppState>) -> anyhow::Result<OpenCodeClient> {
    let conn = state.registry.get_dyn("opencode").await
        .ok_or_else(|| anyhow::anyhow!("opencode connector not available"))?;
    conn.as_any()
        .downcast_ref::<OpenCodeConnector>()
        .ok_or_else(|| anyhow::anyhow!("opencode connector type mismatch"))?
        .client()
        .await
}

pub(crate) async fn affine_from_registry(state: &Arc<AppState>) -> anyhow::Result<AffineClient> {
    let conn = state.registry.get_dyn("affine").await
        .ok_or_else(|| anyhow::anyhow!("affine connector not available"))?;
    conn.as_any()
        .downcast_ref::<AffineConnector>()
        .ok_or_else(|| anyhow::anyhow!("affine connector type mismatch"))?
        .client()
        .await
}
