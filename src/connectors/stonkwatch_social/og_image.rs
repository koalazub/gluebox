use anyhow::{Context, Result};
use std::path::Path;
use tracing::info;

pub struct OgCardData {
    pub symbol: String,
    pub title: String,
    pub ann_type: String,
    pub is_price_sensitive: bool,
    pub sentiment: String,
    pub summary: String,
    pub announcement_id: String,
}

pub async fn generate_og_image(data: &OgCardData, output_dir: &Path) -> Result<std::path::PathBuf> {
    let output_path = output_dir.join(format!("{}.png", data.announcement_id));

    if output_path.exists() {
        return Ok(output_path);
    }

    std::fs::create_dir_all(output_dir).context("Failed to create OG image output directory")?;

    let template_path = find_template()?;

    let summary_truncated = if data.summary.len() > 200 {
        format!("{}...", &data.summary[..197])
    } else {
        data.summary.clone()
    };

    let title_truncated = if data.title.len() > 100 {
        format!("{}...", &data.title[..97])
    } else {
        data.title.clone()
    };

    let status = tokio::process::Command::new("typst")
        .arg("compile")
        .arg(&template_path)
        .arg(&output_path)
        .arg("--input")
        .arg(format!("symbol={}", data.symbol))
        .arg("--input")
        .arg(format!("title={}", title_truncated))
        .arg("--input")
        .arg(format!("type={}", data.ann_type))
        .arg("--input")
        .arg(format!("sentiment={}", data.sentiment))
        .arg("--input")
        .arg(format!("sensitive={}", data.is_price_sensitive))
        .arg("--input")
        .arg(format!("summary={}", summary_truncated))
        .arg("--format")
        .arg("png")
        .output()
        .await
        .context("Failed to run typst")?;

    if !status.status.success() {
        let stderr = String::from_utf8_lossy(&status.stderr);
        anyhow::bail!("Typst render failed: {}", stderr);
    }

    info!(
        symbol = data.symbol,
        path = %output_path.display(),
        "Generated OG image"
    );

    Ok(output_path)
}

fn find_template() -> Result<std::path::PathBuf> {
    let candidates = [
        std::path::PathBuf::from("/etc/gluebox/og-card.typ"),
        std::path::PathBuf::from("assets/og-card.typ"),
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("assets/og-card.typ")))
            .unwrap_or_default(),
    ];

    for path in &candidates {
        if path.exists() {
            return Ok(path.clone());
        }
    }

    anyhow::bail!(
        "OG card template not found. Looked in: {}",
        candidates.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ")
    )
}
