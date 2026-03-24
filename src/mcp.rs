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
struct StudyPlanInput {
    period: String,
    course: Option<String>,
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
                "Gluebox daemon proxy. Manage lecture sessions, study plans, and connectors."
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
