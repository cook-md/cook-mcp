//! MCP tools. `cloud` calls cook.md services; `local` and `pantry` only touch
//! the recipe root.

pub mod cloud;
pub mod local;
pub mod pantry;

use rmcp::model::{CallToolResult, ContentBlock};

pub(crate) fn json_ok(v: &impl serde::Serialize) -> CallToolResult {
    CallToolResult::success(vec![ContentBlock::text(
        serde_json::to_string_pretty(v).unwrap_or_default(),
    )])
}

pub(crate) fn json_err(v: &impl serde::Serialize) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(
        serde_json::to_string_pretty(v).unwrap_or_default(),
    )])
}

pub(crate) fn text_err(msg: impl Into<String>) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(msg.into())])
}

impl crate::server::CookMcp {
    /// `Some(error)` when no recipe folder is configured; local tools return it
    /// as their result instead of touching the filesystem.
    pub(crate) fn unset_guard(&self) -> Option<CallToolResult> {
        self.workspace
            .is_unset()
            .then(|| text_err(crate::workspace::UNSET_HINT))
    }
}

/// A `cookcli-core` failure as a tool error. Parse failures keep their
/// diagnostics and the parser's report so the agent can fix the line.
pub(crate) fn core_err(e: cookcli_core::CoreError) -> CallToolResult {
    match e {
        cookcli_core::CoreError::Parse {
            name,
            diagnostics,
            rendered,
            ..
        } => json_err(&serde_json::json!({
            "error": "parse_failed", "recipe": name, "diagnostics": diagnostics, "report": rendered,
        })),
        other => text_err(other.to_string()),
    }
}

/// A workspace failure as a tool error. Validation failures keep their
/// diagnostics (the agent needs them to fix the content); everything else is
/// its message.
pub(crate) fn workspace_err(e: crate::workspace::WorkspaceError) -> CallToolResult {
    use crate::workspace::WorkspaceError;
    match e {
        WorkspaceError::Invalid { diagnostics } => json_err(&serde_json::json!({
            "error": "invalid_cooklang",
            "message": "Not saved. Fix the errors below (call validate to re-check), or pass force: true.",
            "diagnostics": diagnostics,
        })),
        other => text_err(other.to_string()),
    }
}

#[cfg(test)]
pub(crate) fn text_of(r: &CallToolResult) -> String {
    r.content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect()
}
