use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, RwLock};
use std::time::Duration;

use rmcp::{
    ServerHandler, handler::server::router::tool::ToolRouter, model::*, prompt_handler,
    tool_handler,
};

use crate::api::ApiClient;
use crate::auth::AuthManager;
use crate::config::Config;
use crate::roots::Guards;
use crate::workspace::{RootSource, Workspace};

/// How long to wait for the client's `roots/list` answer before falling back
/// to the working directory.
const ROOTS_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub struct CookMcp {
    pub cfg: Config,
    pub auth: Arc<AuthManager>,
    pub api: Arc<ApiClient>,
    pub roots: Arc<RootState>,
}

/// The recipe root, which can change after startup: without
/// `COOK_RECIPES_DIR`, the client's MCP roots (asked for lazily, again after
/// `notifications/roots/list_changed`) beat the working directory.
pub struct RootState {
    /// The startup choice (env, cwd or unset), used when roots give nothing.
    fallback: Arc<Workspace>,
    current: RwLock<Arc<Workspace>>,
    /// Ask the client for roots before the next local tool call.
    stale: AtomicBool,
    /// Captured during `initialize`, before any tool call can arrive.
    peer: OnceLock<rmcp::Peer<rmcp::RoleServer>>,
    refresh: tokio::sync::Mutex<()>,
    guards: Guards,
}

impl RootState {
    pub fn new(fallback: Workspace, guards: Guards) -> Self {
        let fallback = Arc::new(fallback);
        Self {
            current: RwLock::new(fallback.clone()),
            stale: AtomicBool::new(fallback.source() != RootSource::Env),
            fallback,
            peer: OnceLock::new(),
            refresh: tokio::sync::Mutex::new(()),
            guards,
        }
    }

    fn current(&self) -> Arc<Workspace> {
        self.current
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn mark_stale(&self) {
        if self.fallback.source() != RootSource::Env {
            self.stale.store(true, Ordering::SeqCst);
        }
    }

    /// The recipe root to use now, asking the client for its roots first when
    /// they may have changed.
    pub async fn get(&self) -> Arc<Workspace> {
        if !self.stale.load(Ordering::SeqCst) {
            return self.current();
        }
        let Some(peer) = self.peer.get() else {
            return self.current();
        };
        let _refreshing = self.refresh.lock().await;
        if !self.stale.swap(false, Ordering::SeqCst) {
            return self.current();
        }
        let advertises_roots = peer
            .peer_info()
            .is_some_and(|i| i.capabilities.roots.is_some());
        let ws = if advertises_roots {
            self.ask_client(peer).await
        } else {
            None
        }
        .map(Arc::new)
        .unwrap_or_else(|| self.fallback.clone());
        tracing::info!("recipe root: {} ({:?})", ws.root(), ws.source());
        *self.current.write().unwrap_or_else(|e| e.into_inner()) = ws.clone();
        ws
    }

    #[allow(deprecated)] // roots: deprecated by SEP-2577, still what clients send
    async fn ask_client(&self, peer: &rmcp::Peer<rmcp::RoleServer>) -> Option<Workspace> {
        let roots = match tokio::time::timeout(ROOTS_TIMEOUT, peer.list_roots()).await {
            Ok(Ok(r)) => r.roots,
            Ok(Err(e)) => {
                tracing::warn!("roots/list failed: {e}");
                return None;
            }
            Err(_) => {
                tracing::warn!("roots/list timed out");
                return None;
            }
        };
        let dir = crate::roots::pick_root(roots.iter().map(|r| r.uri.as_str()), &self.guards)?;
        Workspace::with_source(&dir, RootSource::Roots)
            .inspect_err(|e| tracing::warn!("client root {}: {e}", dir.display()))
            .ok()
    }
}

impl CookMcp {
    pub fn new(cfg: Config, workspace: Workspace) -> Self {
        let auth = Arc::new(AuthManager::new(&cfg));
        let api = Arc::new(ApiClient::new(cfg.clone(), auth.clone()));
        Self {
            cfg,
            auth,
            api,
            roots: Arc::new(RootState::new(workspace, Guards::from_env())),
        }
    }

    #[cfg(test)]
    pub(crate) fn set_unset_for_test(&mut self) {
        let mut ws = (*self.roots.fallback).clone();
        ws.set_unset_for_test();
        self.roots = Arc::new(RootState::new(ws, Guards::default()));
    }

    /// The recipe root for this call. Local tools use
    /// [`local_workspace`](Self::local_workspace), which also refuses an
    /// unset root.
    pub(crate) async fn workspace(&self) -> Arc<Workspace> {
        self.roots.get().await
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

    async fn initialize(
        &self,
        request: InitializeRequestParams,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<InitializeResult, rmcp::ErrorData> {
        // As the default does (the protocol version is negotiated by rmcp
        // after this returns), plus keeping the peer so local tools can ask
        // for roots/list.
        context.peer.set_peer_info(request);
        let _ = self.roots.peer.set(context.peer);
        Ok(self.get_info())
    }

    async fn on_roots_list_changed(
        &self,
        _context: rmcp::service::NotificationContext<rmcp::RoleServer>,
    ) {
        self.roots.mark_stale();
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
