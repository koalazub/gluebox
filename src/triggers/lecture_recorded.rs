use std::path::Path;

/// Handle a completed lecture recording: send transcript to Sayl for note extraction,
/// write the notes alongside the transcript, and return the notes content.
pub async fn handle(
    transcript_path: &str,
    sayl_url: &str,
    extraction_prompt: &str,
) -> anyhow::Result<String> {
    // Read transcript
    let transcript = tokio::fs::read_to_string(transcript_path).await
        .map_err(|e| anyhow::anyhow!("failed to read transcript at {transcript_path}: {e}"))?;

    // Return early if empty
    if transcript.trim().is_empty() {
        tracing::info!(transcript_path, "transcript is empty — skipping note extraction");
        return Ok("empty transcript".to_string());
    }

    // Build the user message with prompt + transcript
    let user_content = format!("{extraction_prompt}\n\n---\n\n{transcript}");

    let request_body = serde_json::json!({
        "model": "default",
        "messages": [
            {
                "role": "system",
                "content": "You extract structured lecture notes"
            },
            {
                "role": "user",
                "content": user_content
            }
        ],
        "max_tokens": 4096,
        "kv_bits": 0
    });

    let client = reqwest::Client::new();
    let endpoint = format!("{sayl_url}/v1/chat/completions");

    let response = client
        .post(&endpoint)
        .json(&request_body)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("failed to POST to Sayl at {endpoint}: {e}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("Sayl returned {status}: {body}");
    }

    let response_json: serde_json::Value = response.json().await
        .map_err(|e| anyhow::anyhow!("failed to parse Sayl response JSON: {e}"))?;

    let notes = response_json
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("unexpected Sayl response shape: {response_json}"))?
        .to_string();

    // Write notes alongside the transcript: replace .txt with .notes.md
    let notes_path = Path::new(transcript_path)
        .with_extension("")
        .to_string_lossy()
        .trim_end_matches(".txt")
        .to_string();
    let notes_path = if transcript_path.ends_with(".txt") {
        format!("{}.notes.md", &transcript_path[..transcript_path.len() - 4])
    } else {
        format!("{}.notes.md", notes_path)
    };

    tokio::fs::write(&notes_path, &notes).await
        .map_err(|e| anyhow::anyhow!("failed to write notes to {notes_path}: {e}"))?;

    tracing::info!(
        transcript = transcript_path,
        notes = %notes_path,
        "lecture notes extracted and written"
    );

    Ok(notes)
}
