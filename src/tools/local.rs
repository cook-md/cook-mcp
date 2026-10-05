use rmcp::tool_router;

use crate::server::CookMcp;

#[tool_router(router = local_router, vis = "pub(crate)")]
impl CookMcp {}
