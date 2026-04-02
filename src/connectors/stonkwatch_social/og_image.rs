use anyhow::{Context, Result};
use std::path::Path;
use tracing::info;

pub async fn generate_og_image(
    symbol: &str,
    title: &str,
    ann_type: &str,
    is_price_sensitive: bool,
    summary: &str,
    announcement_id: &str,
    output_dir: &Path,
) -> Result<std::path::PathBuf> {
    let output_path = output_dir.join(format!("{}.png", announcement_id));

    if output_path.exists() {
        return Ok(output_path);
    }

    std::fs::create_dir_all(output_dir).context("Failed to create OG image output directory")?;

    let template_path = find_template()?;

    let summary_truncated = if summary.len() > 200 {
        format!("{}...", &summary[..197])
    } else {
        summary.to_string()
    };

    let title_truncated = if title.len() > 100 {
        format!("{}...", &title[..97])
    } else {
        title.to_string()
    };

    let status = tokio::process::Command::new("typst")
        .arg("compile")
        .arg(&template_path)
        .arg(&output_path)
        .arg("--input")
        .arg(format!("symbol={}", symbol))
        .arg("--input")
        .arg(format!("title={}", title_truncated))
        .arg("--input")
        .arg(format!("type={}", ann_type))
        .arg("--input")
        .arg(format!("sentiment="))
        .arg("--input")
        .arg(format!("sensitive={}", is_price_sensitive))
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
        symbol = symbol,
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
