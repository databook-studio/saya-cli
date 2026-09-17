//! The model-facing anchored edit (`replace` variant only): resolves
//! `old_text` to exactly one byte range of a contained workspace file and
//! commits the splice through `Workspace::patch_range`. Path resolution, the
//! no-follow open, and the atomic commit are the harness's — never
//! re-derived here. This tool's own jobs are argument typing, the byte
//! bounds, the `expected_*` precondition, and typed refusals that carry
//! counts, line numbers, sizes and digests — never file content.

use saya_agent::ToolError;

use super::DatabaseTools;
use super::workspace_edit_anchor::{find_matches, hex_digest, match_lines};
use super::workspace_write::WORKSPACE_WRITE_MAX_BYTES;

/// The bound on a workspace edit's `old_text` and `new_text`: the same 64
/// KiB round-trip discipline as the whole-file write, so anything an edit
/// inserts can be read back whole. Over the bound is a typed whole-refusal,
/// never a truncation.
pub(crate) const WORKSPACE_EDIT_MAX_BYTES: usize = WORKSPACE_WRITE_MAX_BYTES;

/// The bound on a workspace edit's target file: the harness reads the file
/// whole to anchor the match, so targets are capped at the patch layer's
/// file cap with a typed refusal naming the size and the cap. Larger files
/// stay editable through the process lanes under their own gates.
pub(crate) const WORKSPACE_EDIT_MAX_FILE_BYTES: u64 =
    saya_harness::workspace::patch::PATCH_MAX_FILE_BYTES;

impl DatabaseTools {
    /// Replaces the single occurrence of `old_text` in the contained file
    /// `path` with `new_text`, atomically: zero or multiple matches refuse
    /// with a typed error and write nothing — never "first wins". The
    /// optional `expected_size`/`expected_digest` precondition refuses with a
    /// typed error when the file's current state no longer matches what the
    /// model measured, so a moved anchor cannot splice against stale offsets.
    /// Like the workspace read tools this never touches a connection —
    /// dispatch routes it before connection resolution — and it records
    /// nothing into the knowledge store: a workspace edit is not evidence
    /// about a database object. With no workspace attached it denies with a
    /// typed error: no workspace, no edit.
    pub(super) async fn workspace_edit(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        let rel = arguments
            .get("path")
            .and_then(serde_json::Value::as_str)
            .ok_or(ToolError::PathNotString)?;
        let old_text = arguments
            .get("old_text")
            .and_then(serde_json::Value::as_str)
            .ok_or(ToolError::OldTextNotString)?;
        let new_text = arguments
            .get("new_text")
            .and_then(serde_json::Value::as_str)
            .ok_or(ToolError::NewTextNotString)?;
        let expected_size = match arguments.get("expected_size") {
            None | Some(serde_json::Value::Null) => None,
            Some(value) => Some(value.as_u64().ok_or(ToolError::ExpectedSizeNotUint)?),
        };
        let expected_digest = match arguments.get("expected_digest") {
            None | Some(serde_json::Value::Null) => None,
            Some(value) => Some(
                value
                    .as_str()
                    .ok_or(ToolError::ExpectedDigestNotString)?
                    .to_owned(),
            ),
        };
        if old_text.is_empty() {
            return Err(ToolError::WorkspaceEditEmptyAnchor {
                path: rel.to_string(),
            });
        }
        if old_text.len() > WORKSPACE_EDIT_MAX_BYTES {
            return Err(ToolError::WorkspaceEditTooLarge {
                path: rel.to_string(),
                limit: WORKSPACE_EDIT_MAX_BYTES,
                found: old_text.len(),
            });
        }
        if new_text.len() > WORKSPACE_EDIT_MAX_BYTES {
            return Err(ToolError::WorkspaceEditTooLarge {
                path: rel.to_string(),
                limit: WORKSPACE_EDIT_MAX_BYTES,
                found: new_text.len(),
            });
        }
        let Some(workspace) = &self.workspace else {
            return Err(ToolError::WorkspaceUnavailable);
        };
        let target_size = workspace_size(workspace, rel)?;
        if target_size > WORKSPACE_EDIT_MAX_FILE_BYTES {
            return Err(ToolError::WorkspaceEdit(
                saya_harness::HarnessError::BoundsExceeded {
                    path: rel.to_string(),
                    found: target_size,
                    max: WORKSPACE_EDIT_MAX_FILE_BYTES,
                }
                .to_string(),
            ));
        }
        let file = workspace
            .read(rel, WORKSPACE_EDIT_MAX_FILE_BYTES)
            .map_err(|error| ToolError::WorkspaceEdit(error.to_string()))?;
        let current = match std::str::from_utf8(&file.bytes) {
            Ok(text) => text,
            Err(_) => {
                return Err(ToolError::WorkspaceEditNotText {
                    path: rel.to_string(),
                });
            }
        };
        let size = file.size;
        let digest = hex_digest(&file.bytes);
        if let Some(want) = expected_size
            && want != size
        {
            return Err(ToolError::WorkspaceEditMoved {
                path: rel.to_string(),
                expected_size: Some(want),
                current_size: size,
                expected_digest: expected_digest.clone(),
                current_digest: digest,
            });
        }
        if let Some(want) = expected_digest.as_deref()
            && want != digest
        {
            return Err(ToolError::WorkspaceEditMoved {
                path: rel.to_string(),
                expected_size,
                current_size: size,
                expected_digest: expected_digest.clone(),
                current_digest: digest,
            });
        }
        let hits = find_matches(current, old_text);
        if hits.is_empty() {
            return Err(ToolError::WorkspaceEditNoMatch {
                path: rel.to_string(),
                matches: 0,
                size,
                digest,
            });
        }
        if hits.len() > 1 {
            let lines = match_lines(current, &hits);
            return Err(ToolError::WorkspaceEditAmbiguous {
                path: rel.to_string(),
                matches: hits.len(),
                lines,
                size,
                digest,
            });
        }
        let start = hits[0] as u64;
        let end = start + old_text.len() as u64;
        workspace
            .patch_range(rel, start..end, size, new_text.as_bytes())
            .map_err(|error| ToolError::WorkspaceEdit(error.to_string()))?;
        let after = workspace
            .read(rel, WORKSPACE_EDIT_MAX_FILE_BYTES)
            .map_err(|error| ToolError::WorkspaceEdit(error.to_string()))?;
        Ok(serde_json::json!({
            "path": rel,
            "bytes_replaced": old_text.len(),
            "bytes_written": new_text.len(),
            "size": after.size,
            "digest": hex_digest(&after.bytes),
        }))
    }
}

/// The target's pre-read size probe: the contained scan behind
/// [`Workspace::read`] names the size even when the content would be capped,
/// and this probe keeps the over-cap refusal typed without serving bytes the
/// edit never needs. A missing or refused name surfaces as the edit's own
/// containment error.
fn workspace_size(
    workspace: &saya_harness::workspace::Workspace,
    rel: &str,
) -> Result<u64, ToolError> {
    let probe = workspace
        .read(rel, 0)
        .map_err(|error| ToolError::WorkspaceEdit(error.to_string()))?;
    Ok(probe.size)
}
