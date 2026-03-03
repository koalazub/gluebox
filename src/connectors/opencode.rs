use reqwest;
use serde::Deserialize;
use serde_json::json;

pub struct OpenCodeClient {
    client: reqwest::Client,
    api_key: String,
    base_url: String,
    model: String,
}

#[derive(Debug, Deserialize)]
pub struct ChatCompletionResponse {
    pub choices: Vec<Choice>,
}

#[derive(Debug, Deserialize)]
pub struct Choice {
    pub message: Message,
    #[allow(dead_code)]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Message {
    pub content: Option<String>,
    pub reasoning_content: Option<String>,
    #[allow(dead_code)]
    pub role: String,
}

impl OpenCodeClient {
    pub fn new(api_key: &str) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key: api_key.to_string(),
            base_url: "https://opencode.ai/zen/v1".to_string(),
            model: "trinity-large-preview-free".to_string(),
        }
    }

    pub async fn chat(&self, system: &str, user: &str, max_tokens: u32) -> anyhow::Result<String> {
        let url = format!("{}/chat/completions", self.base_url);
        
        let payload = json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": user}
            ],
            "max_tokens": max_tokens,
            "temperature": 0.7,
        });

        let resp = self.client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await?;
            if status.as_u16() == 429 {
                anyhow::bail!("Rate limited - try again in a minute");
            }
            anyhow::bail!("AI error ({}): {}", status, text);
        }

        let text = resp.text().await?;
        let data: ChatCompletionResponse = serde_json::from_str(&text)
            .map_err(|e| anyhow::anyhow!("Failed to parse AI response: {e}\nRaw: {}", &text[..text.len().min(200)]))?;
        
        let content = data.choices
            .first()
            .and_then(|c| c.message.content.as_ref())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .or_else(|| {
                data.choices
                    .first()
                    .and_then(|c| c.message.reasoning_content.as_ref())
                    .map(|s| s.trim().to_string())
            })
            .unwrap_or_else(|| "(no response)".to_string());

        Ok(content)
    }

    pub async fn draft_spec(&self, prompt: &str) -> anyhow::Result<String> {
        let system = r#"You are a technical specification writer. 
Create a concise technical spec with the following structure:
1. Overview - What this is and why it matters (2-3 sentences)
2. Goals - Bullet points of what this achieves
3. Technical Approach - High-level architecture and key decisions
4. Implementation Notes - Specific technical details, APIs, data models
5. Open Questions - What needs clarification

Be specific, actionable, and developer-focused. Use markdown formatting."#;

        self.chat(system, prompt, 4000).await
    }

    pub async fn draft_decision(&self, context: &str) -> anyhow::Result<String> {
        let system = r#"You are an architecture decision record (ADR) writer.
Create an ADR with the following structure:
1. Title - Clear statement of the decision
2. Status - Proposed
3. Context - What problem are we solving and what forces are at play
4. Decision - The decision we made
5. Consequences - What becomes easier/harder because of this decision

Use markdown. Be concise but thorough."#;

        self.chat(system, context, 3000).await
    }

    pub async fn draft_issue(&self, description: &str) -> anyhow::Result<(String, String)> {
        let system = r#"You are a project manager creating Linear issues.
Given a description, output ONLY a JSON object with:
- "title": A clear, actionable issue title (under 80 chars)
- "description": A detailed description with acceptance criteria

Do not include markdown code blocks, just the raw JSON."#;

        let response = self.chat(system, description, 2000).await?;
        
        let cleaned = response
            .trim()
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim();

        let parsed: serde_json::Value = serde_json::from_str(cleaned)?;
        let title = parsed["title"].as_str().unwrap_or("(untitled)").to_string();
        let description = parsed["description"].as_str().unwrap_or("").to_string();
        
        Ok((title, description))
    }

    pub async fn classify_intent(&self, message: &str) -> anyhow::Result<Intent> {
        let system = r#"You classify user messages into one of these intents. Output ONLY a JSON object, no markdown.

Intents:
- "spec": user wants to draft a technical specification or design doc
- "decision": user wants to write an architecture decision record (ADR) or make a technical decision
- "issue": user wants to create a task, ticket, or issue in the project tracker
- "chat": anything else - general question, greeting, or conversation

Output format: {"intent": "<spec|decision|issue|chat>", "prompt": "<the actual request, cleaned up>"}

Examples:
- "draft a spec for webhook retry logic" -> {"intent": "spec", "prompt": "webhook retry logic"}
- "we need to decide whether to use postgres or sqlite" -> {"intent": "decision", "prompt": "whether to use postgres or sqlite"}
- "can you file a ticket for fixing the login bug" -> {"intent": "issue", "prompt": "fix the login bug"}
- "what's the status of the deploy" -> {"intent": "chat", "prompt": "what's the status of the deploy"}"#;

        let response = self.chat(system, message, 500).await?;

        let cleaned = response
            .trim()
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim();

        let parsed: serde_json::Value = serde_json::from_str(cleaned)
            .unwrap_or_else(|_| serde_json::json!({"intent": "chat", "prompt": message}));

        let intent_str = parsed["intent"].as_str().unwrap_or("chat");
        let prompt = parsed["prompt"].as_str().unwrap_or(message).to_string();

        let kind = match intent_str {
            "spec" => IntentKind::Spec,
            "decision" => IntentKind::Decision,
            "issue" => IntentKind::Issue,
            "feedback" => IntentKind::Feedback,
            _ => IntentKind::Chat,
        };

        Ok(Intent { kind, prompt })
    }

    pub async fn chat_reply(&self, message: &str) -> anyhow::Result<String> {
        let system = r#"You are a helpful engineering assistant in a team chat room. Keep replies concise and technical. Use markdown formatting."#;
        self.chat(system, message, 2000).await
    }

    /// Extract discrete feedback items from a message, cluster similar ones,
    /// and categorise each cluster. Returns an array ready to file as tickets.
    pub async fn extract_and_cluster_feedback(
        &self,
        message: &str,
    ) -> anyhow::Result<Vec<FeedbackCluster>> {
        let system = r#"You are a product feedback analyst. Extract discrete feedback items from the user message, cluster similar ones together, and categorise each cluster.

Output ONLY a JSON array (no markdown, no code fences). Each element must have:
- "title": short actionable title under 80 chars, suitable for a Linear issue
- "description": detailed description with what the user wants and why, suitable for a Linear issue body
- "category": exactly one of: "bug" | "feature" | "ux" | "performance" | "docs" | "other"
- "items": array of verbatim or closely paraphrased individual feedback points in this cluster

Rules:
- Merge feedback items that describe the same root problem into one cluster
- Do NOT merge unrelated feedback just to reduce count
- If there is only one distinct topic, return a single-element array
- Ignore greetings, meta-commentary, and off-topic content"#;

        let response = self.chat(system, message, 3000).await?;
        let cleaned = response
            .trim()
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim();

        let clusters: Vec<FeedbackCluster> = serde_json::from_str(cleaned)
            .map_err(|e| anyhow::anyhow!("failed to parse feedback clusters: {e}\nRaw: {}", &cleaned[..cleaned.len().min(400)]))?;

        Ok(clusters)
    }

    /// Given a new cluster title+description and a list of existing issue
    /// summaries, return the ID of the matching existing issue if this is a
    /// duplicate, or None if it is genuinely new.
    pub async fn find_duplicate_issue(
        &self,
        cluster: &FeedbackCluster,
        existing: &[ExistingIssueSummary],
    ) -> anyhow::Result<Option<String>> {
        if existing.is_empty() {
            return Ok(None);
        }

        let existing_json = serde_json::to_string(existing)?;

        let user_prompt = format!(
            "New feedback cluster:\nTitle: {}\nDescription: {}\nCategory: {}\n\nExisting issues:\n{}",
            cluster.title, cluster.description, cluster.category, existing_json
        );

        let system = r#"You are deduplicating product feedback against existing Linear issues.

Given a new feedback cluster and a list of existing issues, determine if the new cluster describes the SAME root problem as any existing issue.

Criteria for a duplicate:
- Same underlying user pain point or request (even if described differently)
- Same category
- A comment on the existing issue would be more appropriate than a new ticket

Output ONLY a JSON object (no markdown):
- {"duplicate": true, "id": "<issue_id>"} if it matches an existing issue
- {"duplicate": false} if it is genuinely new

Be conservative: only mark as duplicate when confident. Different symptoms of the same system can be separate issues."#;

        let response = self.chat(system, &user_prompt, 300).await?;
        let cleaned = response
            .trim()
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim();

        let parsed: serde_json::Value = serde_json::from_str(cleaned)
            .unwrap_or_else(|_| serde_json::json!({"duplicate": false}));

        if parsed["duplicate"].as_bool().unwrap_or(false) {
            Ok(parsed["id"].as_str().map(|s| s.to_string()))
        } else {
            Ok(None)
        }
    }
}

/// Lightweight summary of an existing Linear issue used for deduplication.
#[derive(Debug, serde::Serialize)]
pub struct ExistingIssueSummary {
    pub id: String,
    pub title: String,
    pub category: String,
}

pub struct Intent {
    pub kind: IntentKind,
    pub prompt: String,
}

#[derive(Debug)]
pub enum IntentKind {
    Spec,
    Decision,
    Issue,
    Feedback,
    Chat,
}

/// A cluster of related feedback items extracted from one or more messages.
#[derive(Debug, serde::Deserialize)]
pub struct FeedbackCluster {
    pub title: String,
    pub description: String,
    /// One of: "bug" | "feature" | "ux" | "performance" | "docs" | "other"
    pub category: String,
    pub items: Vec<String>,
}
