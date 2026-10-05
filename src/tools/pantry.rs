use rmcp::tool_router;

use crate::server::CookMcp;

#[tool_router(router = pantry_router, vis = "pub(crate)")]
impl CookMcp {}
