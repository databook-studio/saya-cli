//! The one file-reading tool: a contained, bounded read from the run
//! workspace. Path resolution is delegated entirely to
//! `saya_harness::workspace::Workspace::read` — validation, symlink refusal,
//! and the prefix check are the harness's, never re-derived here. A failure is
//! surfaced as the harness error's own text, so the model reads the real
//! containment reason rather than a guess or an empty result.

use saya_agent::ToolError;

use super::DatabaseTools;

/// The read bound passed to the workspace. A larger file is truncated at this
/// many bytes and the truncation is reported in the result — never silent.
pub(crate) const WORKSPACE_READ_MAX_BYTES: u64 = 64 * 1024;

impl DatabaseTools {
    /// Reads one file from the run workspace under containment. This tool
    /// never touches a connection — a workspace-only run has no selected
    /// profile — so dispatch routes it before connection resolution (see
    /// `dispatch`). With no workspace attached it denies with a typed error:
    /// no workspace, no read.
    pub(super) async fn workspace_read(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        let rel = arguments
            .get("path")
            .and_then(serde_json::Value::as_str)
            .ok_or(ToolError::PathNotString)?;
        let Some(workspace) = &self.workspace else {
            return Err(ToolError::WorkspaceUnavailable);
        };
        let file = workspace
            .read(rel, WORKSPACE_READ_MAX_BYTES)
            .map_err(|error| ToolError::Workspace(error.to_string()))?;
        // Content is served as text (lossy for stray non-UTF-8 bytes); `size`
        // is the file's full size and `truncated` says whether `content` was
        // capped at the bound, so the model can tell a prefix from the file.
        // `digest` is the sha256 of the file's whole bytes — hashed in the
        // same contained open — so a truncated read still names the state an
        // `expected_digest` edit precondition can state.
        Ok(serde_json::json!({
            "path": rel,
            "size": file.size,
            "truncated": file.truncated,
            "digest": file.digest,
            "content": String::from_utf8_lossy(&file.bytes),
        }))
    }
}
