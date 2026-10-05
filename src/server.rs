use std::sync::Arc;

use rmcp::{
    ServerHandler, handler::server::router::tool::ToolRouter, model::*, prompt_handler,
    tool_handler,
};

use crate::api::ApiClient;
use crate::auth::AuthManager;
use crate::config::Config;
use crate::workspace::Workspace;

#[derive(Clone)]
pub struct CookMcp {
    pub cfg: Config,
    pub auth: Arc<AuthManager>,
    pub api: Arc<ApiClient>,
    pub workspace: Arc<Workspace>,
}

impl CookMcp {
    pub fn new(cfg: Config, workspace: Workspace) -> Self {
        let auth = Arc::new(AuthManager::new(&cfg));
        let api = Arc::new(ApiClient::new(cfg.clone(), auth.clone()));
        Self {
            cfg,
            auth,
            api,
            workspace: Arc::new(workspace),
        }
    }

    /// Every tool, free and cloud. Built per call by the handler macro; it's a
    /// handful of map inserts.
    pub(crate) fn all_tools() -> ToolRouter<Self> {
        Self::local_router() + Self::pantry_router() + Self::cloud_router()
    }
}

#[tool_handler(router = Self::all_tools())]
#[prompt_handler]
impl ServerHandler for CookMcp {
    fn get_info(&self) -> ServerInfo {
        // ServerInfo is #[non_exhaustive] in rmcp 2.2: use new() + with_instructions().
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_prompts()
                .enable_resources()
                .build(),
        )
        .with_server_info(Implementation::new("cook-mcp", env!("CARGO_PKG_VERSION")))
        .with_instructions(crate::knowledge::INSTRUCTIONS)
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<ListResourcesResult, rmcp::ErrorData> {
        Ok(ListResourcesResult::with_all_items(crate::knowledge::list()))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<ReadResourceResult, rmcp::ErrorData> {
        match crate::knowledge::read(&request.uri) {
            Some(text) => Ok(ReadResourceResult::new(vec![
                ResourceContents::text(text, request.uri).with_mime_type("text/markdown"),
            ])),
            None => Err(rmcp::ErrorData::resource_not_found(
                format!("unknown resource {}", request.uri),
                None,
            )),
        }
    }
}
