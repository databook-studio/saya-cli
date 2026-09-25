//! The one file-writing tool: an atomic, bounded write into the run
//! workspace. Path validation, symlink refusal, the prefix check, and the
//! temp-then-rename write itself are the harness's
//! (`saya_harness::workspace::Workspace::write`) — never re-derived here. This
//! tool's own jobs are argument typing, the byte bound, and surfacing the
//! harness's containment reason as the error text.

use saya_agent::ToolError;

use super::DatabaseTools;
use super::redaction_guard;

/// The bound on a workspace write's `content`, matched to
/// `WORKSPACE_READ_MAX_BYTES` (64 KiB) so anything this tool writes can be
/// read back whole in a single `workspace_read` call — the workspace
/// round-trips at most this many bytes through the model's tools in either
/// direction. It also keeps the call's serialized arguments comfortably
/// within every provider's per-message tool-call budget, so a write is never
/// the call the provider silently clips. Over the bound is a typed refusal —
/// never a truncated write: a partially written file is worse than a refused
/// one, and the model can split large content across several files instead.
pub(crate) const WORKSPACE_WRITE_MAX_BYTES: usize = 64 * 1024;

impl DatabaseTools {
    /// Writes one file into the run workspace under containment. Like the
    /// workspace read tools this never touches a connection — dispatch routes
    /// it before connection resolution — and it records nothing into the
    /// knowledge store: a workspace write is not evidence about a database
    /// object. With no workspace attached it denies with a typed error: no
    /// workspace, no write.
    pub(super) async fn workspace_write(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        let rel = arguments
            .get("path")
            .and_then(serde_json::Value::as_str)
            .ok_or(ToolError::PathNotString)?;
        let content = arguments
            .get("content")
            .and_then(serde_json::Value::as_str)
            .ok_or(ToolError::ContentNotString)?;
        let Some(workspace) = &self.workspace else {
            return Err(ToolError::WorkspaceUnavailable);
        };
        if content.len() > WORKSPACE_WRITE_MAX_BYTES {
            return Err(ToolError::WorkspaceWriteTooLarge {
                limit: WORKSPACE_WRITE_MAX_BYTES,
            });
        }
        // The redaction-placeholder guard: refuse before any byte is
        // written when this write would grow the file's `[redacted]` count
        // past what is already on disk (0 for a file that doesn't exist
        // yet) — the model must not be able to copy a masked tool result
        // back over the real secret it was standing in for.
        let existing_markers = redaction_guard::existing_marker_count(workspace, rel)
            .map_err(|error| ToolError::WorkspaceWrite(error.to_string()))?;
        redaction_guard::refuse_marker_growth(
            rel,
            existing_markers,
            content.as_bytes(),
            ToolError::WorkspaceWrite,
        )?;
        workspace
            .write(rel, content.as_bytes())
            .map_err(|error| ToolError::WorkspaceWrite(error.to_string()))?;
        Ok(serde_json::json!({
            "path": rel,
            "bytes_written": content.len(),
        }))
    }
}
