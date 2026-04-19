use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use tokio::process::Command;

pub async fn concat_mp4s(inputs: &[PathBuf], output_path: &Path) -> Result<()> {
    if inputs.is_empty() {
        anyhow::bail!("no inputs to stitch");
    }
    let tmp_dir = tempfile::tempdir().context("tempdir for ffmpeg list")?;
    let list_path = tmp_dir.path().join("inputs.txt");
    let list_content = inputs
        .iter()
        .map(|p| format!("file '{}'", p.to_string_lossy().replace('\'', "'\\''")))
        .collect::<Vec<_>>()
        .join("\n");
    tokio::fs::write(&list_path, list_content)
        .await
        .context("write ffmpeg list")?;

    let status = Command::new("ffmpeg")
        .args(["-y", "-f", "concat", "-safe", "0"])
        .arg("-i")
        .arg(&list_path)
        .args(["-c", "copy"])
        .arg(output_path)
        .output()
        .await
        .context("spawn ffmpeg")?;

    if !status.status.success() {
        let stderr = String::from_utf8_lossy(&status.stderr);
        anyhow::bail!("ffmpeg concat failed: {}", stderr);
    }
    Ok(())
}
