use rmcp::{
    ErrorData as McpError, ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::*,
    schemars, tool, tool_handler, tool_router,
    transport::io::stdio,
};

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct ImportSessionInput {
    session_id: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct SocialPostInput {
    /// The post text to publish
    text: String,
    /// Platforms to post to: "x", "bluesky", "instagram", "facebook". Defaults to all configured.
    platforms: Option<Vec<String>>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct GenerateSocialInput {
    /// Optional: filter by stock symbol (e.g. "BHP")
    symbol: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct StudyPlanInput {
    period: String,
    course: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct CreateDocInput {
    title: String,
    markdown: String,
    /// Affine workspace name (e.g. "default", "stonkington"). Omit for default.
    workspace: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct ToggleConnectorInput {
    name: String,
}

#[derive(Clone)]
struct GlueboxMcp {
    client: reqwest::Client,
    base_url: String,
    auth_token: String,
    tool_router: ToolRouter<Self>,
}

impl GlueboxMcp {
    fn auth_header(&self) -> String {
        format!("Bearer {}", self.auth_token)
    }
}

#[tool_router]
impl GlueboxMcp {
    fn new(base_url: String, auth_token: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url,
            auth_token,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(description = "List imported lecture sessions")]
    async fn list_sessions(&self) -> Result<CallToolResult, McpError> {
        let resp = self
            .client
            .get(format!("{}/api/sessions", self.base_url))
            .header("Authorization", self.auth_header())
            .send()
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let body = resp
            .text()
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(body)]))
    }

    #[tool(description = "Import latest unimported uni session")]
    async fn import_latest(&self) -> Result<CallToolResult, McpError> {
        let resp = self
            .client
            .post(format!("{}/api/import", self.base_url))
            .header("Authorization", self.auth_header())
            .send()
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let body = resp
            .text()
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(body)]))
    }

    #[tool(description = "Batch import all unimported uni sessions")]
    async fn import_all(&self) -> Result<CallToolResult, McpError> {
        let resp = self
            .client
            .post(format!("{}/api/import/all", self.base_url))
            .header("Authorization", self.auth_header())
            .send()
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let body = resp
            .text()
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(body)]))
    }

    #[tool(description = "Import specific session by ID")]
    async fn import_session(
        &self,
        Parameters(input): Parameters<ImportSessionInput>,
    ) -> Result<CallToolResult, McpError> {
        let resp = self
            .client
            .post(format!(
                "{}/api/import/{}",
                self.base_url, input.session_id
            ))
            .header("Authorization", self.auth_header())
            .send()
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let body = resp
            .text()
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(body)]))
    }

    #[tool(description = "Generate study plan for a period")]
    async fn create_study_plan(
        &self,
        Parameters(input): Parameters<StudyPlanInput>,
    ) -> Result<CallToolResult, McpError> {
        let mut body = serde_json::json!({ "period": input.period });
        if let Some(course) = input.course {
            body["course"] = serde_json::Value::String(course);
        }
        let resp = self
            .client
            .post(format!("{}/api/study-plan", self.base_url))
            .header("Authorization", self.auth_header())
            .json(&body)
            .send()
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let text = resp
            .text()
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    #[tool(description = "Create a document in AFFine. Use 'workspace' to target a specific workspace (e.g. 'default', 'stonkington'). Omit for the default workspace.")]
    async fn create_document(
        &self,
        Parameters(input): Parameters<CreateDocInput>,
    ) -> Result<CallToolResult, McpError> {
        let mut body = serde_json::json!({
            "title": input.title,
            "markdown": input.markdown,
        });
        if let Some(ref ws) = input.workspace {
            body["workspace"] = serde_json::json!(ws);
        }
        let resp = self
            .client
            .post(format!("{}/api/doc", self.base_url))
            .header("Authorization", self.auth_header())
            .json(&body)
            .send()
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let text = resp
            .text()
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    #[tool(description = "List all connectors with status")]
    async fn connector_status(&self) -> Result<CallToolResult, McpError> {
        let resp = self
            .client
            .get(format!("{}/admin/connectors", self.base_url))
            .header("Authorization", self.auth_header())
            .send()
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let body = resp
            .text()
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(body)]))
    }

    #[tool(description = "Toggle a connector on or off")]
    async fn toggle_connector(
        &self,
        Parameters(input): Parameters<ToggleConnectorInput>,
    ) -> Result<CallToolResult, McpError> {
        let resp = self
            .client
            .post(format!(
                "{}/admin/connectors/{}/toggle",
                self.base_url, input.name
            ))
            .header("Authorization", self.auth_header())
            .send()
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let body = resp
            .text()
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(body)]))
    }

    #[tool(description = "Generate social media posts from recent ASX announcements. Uses AI to create engaging, platform-aware content. Optionally filter by stock symbol.")]
    async fn generate_social_posts(
        &self,
        Parameters(input): Parameters<GenerateSocialInput>,
    ) -> Result<CallToolResult, McpError> {
        let url = format!("{}/api/social/generate", self.base_url);
        let body = serde_json::json!({ "symbol": input.symbol });
        let resp = self
            .client
            .post(&url)
            .header("Authorization", self.auth_header())
            .json(&body)
            .send()
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let text = resp
            .text()
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    #[tool(description = "Publish a social media post to X, Bluesky, Instagram, and/or Facebook. Provide the post text and optionally specify which platforms.")]
    async fn publish_social_post(
        &self,
        Parameters(input): Parameters<SocialPostInput>,
    ) -> Result<CallToolResult, McpError> {
        let body = serde_json::json!({
            "text": input.text,
            "platforms": input.platforms,
        });
        let resp = self
            .client
            .post(format!("{}/api/social/post", self.base_url))
            .header("Authorization", self.auth_header())
            .json(&body)
            .send()
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let text = resp
            .text()
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    #[tool(description = "Generate AI-powered social posts from recent ASX announcements and publish them to all configured platforms (X, Bluesky). One-shot: fetches data, generates content, posts.")]
    async fn generate_and_post_all(&self) -> Result<CallToolResult, McpError> {
        let resp = self
            .client
            .post(format!("{}/api/social/post-all", self.base_url))
            .header("Authorization", self.auth_header())
            .send()
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let text = resp
            .text()
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    #[tool(description = "Hot-reload daemon configuration")]
    async fn reload_config(&self) -> Result<CallToolResult, McpError> {
        let resp = self
            .client
            .post(format!("{}/admin/reload", self.base_url))
            .header("Authorization", self.auth_header())
            .send()
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let body = resp
            .text()
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(body)]))
    }
}

#[tool_handler]
impl ServerHandler for GlueboxMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("gluebox", env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "Gluebox daemon proxy. Manage lecture sessions, study plans, connectors, and Stonkwatch social media posting (X, Bluesky, Instagram, Facebook)."
                    .to_string(),
            )
    }
}

pub async fn run(base_url: String, auth_token: String) -> anyhow::Result<()> {
    let mcp = GlueboxMcp::new(base_url, auth_token);
    let service = mcp.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
