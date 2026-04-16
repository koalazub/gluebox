use std::sync::Arc;
use crate::AppState;

pub async fn generate_plan(_state: &Arc<AppState>, _period: &str, _course: Option<&str>) -> anyhow::Result<String> {
    anyhow::bail!("study plan generation is not available (Char/Hyprnote integration removed)")
}
