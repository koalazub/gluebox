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

    let template_path = find_template("og-card.typ")?;
    render_typst(&template_path, &output_path, symbol, title, ann_type, is_price_sensitive, summary, None).await?;

    info!(symbol, path = %output_path.display(), "Generated OG image");
    Ok(output_path)
}

pub async fn generate_story_image(
    symbol: &str,
    title: &str,
    ann_type: &str,
    is_price_sensitive: bool,
    summary: &str,
    announcement_id: &str,
    link: &str,
    output_dir: &Path,
) -> Result<std::path::PathBuf> {
    let output_path = output_dir.join(format!("{}-story.png", announcement_id));

    if output_path.exists() {
        return Ok(output_path);
    }

    let template_path = find_template("story-card.typ")?;
    render_typst(&template_path, &output_path, symbol, title, ann_type, is_price_sensitive, summary, Some(link)).await?;

    info!(symbol, path = %output_path.display(), "Generated story image");
    Ok(output_path)
}

async fn render_typst(
    template: &Path,
    output: &Path,
    symbol: &str,
    title: &str,
    ann_type: &str,
    is_price_sensitive: bool,
    summary: &str,
    link: Option<&str>,
) -> Result<()> {
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent).context("Failed to create output directory")?;
    }

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

    let mut cmd = tokio::process::Command::new("typst");
    cmd.arg("compile")
        .arg(template)
        .arg(output)
        .arg("--input").arg(format!("symbol={}", symbol))
        .arg("--input").arg(format!("title={}", title_truncated))
        .arg("--input").arg(format!("type={}", ann_type))
        .arg("--input").arg(format!("sensitive={}", is_price_sensitive))
        .arg("--input").arg(format!("summary={}", summary_truncated))
        .arg("--format").arg("png");

    if let Some(l) = link {
        cmd.arg("--input").arg(format!("link={}", l));
    }

    let status = cmd.output().await.context("Failed to run typst")?;

    if !status.status.success() {
        let stderr = String::from_utf8_lossy(&status.stderr);
        anyhow::bail!("Typst render failed: {}", stderr);
    }

    Ok(())
}

fn find_template(name: &str) -> Result<std::path::PathBuf> {
    let candidates = [
        std::path::PathBuf::from(format!("/etc/gluebox/{}", name)),
        std::path::PathBuf::from(format!("assets/{}", name)),
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join(format!("assets/{}", name))))
            .unwrap_or_default(),
    ];

    for path in &candidates {
        if path.exists() {
            return Ok(path.clone());
        }
    }

    anyhow::bail!(
        "Template {} not found. Looked in: {}",
        name,
        candidates.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ")
    )
}
