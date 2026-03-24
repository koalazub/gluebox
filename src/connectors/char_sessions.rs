use std::path::{Path, PathBuf};
use chrono::{DateTime, Utc};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct SessionMeta {
    pub id: String,
    pub title: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ParsedSession {
    pub meta: SessionMeta,
    pub summary: String,
    pub dir: PathBuf,
}

pub fn list_session_dirs(sessions_dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(sessions_dir) else {
        return Vec::new();
    };
    entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect()
}

pub fn parse_session(dir: &Path) -> anyhow::Result<ParsedSession> {
    let meta_path = dir.join("meta.json");
    let meta_content = std::fs::read_to_string(&meta_path)
        .map_err(|e| anyhow::anyhow!("failed to read {}: {e}", meta_path.display()))?;
    let meta: SessionMeta = serde_json::from_str(&meta_content)
        .map_err(|e| anyhow::anyhow!("failed to parse {}: {e}", meta_path.display()))?;

    let summary_path = dir.join("_summary.md");
    let summary = if summary_path.exists() {
        let raw = std::fs::read_to_string(&summary_path)?;
        strip_frontmatter(&raw)
    } else {
        String::new()
    };

    Ok(ParsedSession {
        meta,
        summary,
        dir: dir.to_path_buf(),
    })
}

pub fn strip_frontmatter(content: &str) -> String {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return content.to_string();
    }
    let after_first = &trimmed[3..];
    if let Some(end) = after_first.find("---") {
        after_first[end + 3..].trim_start().to_string()
    } else {
        content.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_frontmatter_removes_yaml_header() {
        let input = "---\ntitle: Test\n---\nBody content";
        assert_eq!(strip_frontmatter(input), "Body content");
    }

    #[test]
    fn strip_frontmatter_no_header() {
        let input = "Just some content";
        assert_eq!(strip_frontmatter(input), "Just some content");
    }

    #[test]
    fn strip_frontmatter_unclosed_header() {
        let input = "---\ntitle: Test\nBody content";
        assert_eq!(strip_frontmatter(input), input);
    }
}
