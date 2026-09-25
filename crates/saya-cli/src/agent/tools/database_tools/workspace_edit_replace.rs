use saya_agent::ToolError;

use super::workspace_edit::{WORKSPACE_EDIT_MAX_BYTES, WORKSPACE_EDIT_MAX_FILE_BYTES};
use super::workspace_edit_anchor::{find_matches, hex_digest, match_lines};
use super::workspace_edit_shared::{commit_splice, splice_text, workspace_size};
use super::{DatabaseTools, redaction_guard};

impl DatabaseTools {
    /// The `replace` half of [`DatabaseTools::workspace_edit`]: resolve
    /// `old_text` to exactly one byte range, then commit the splice through
    /// `Workspace::patch_range`.
    pub(super) async fn workspace_replace(
        &self,
        rel: &str,
        old_text: &str,
        new_text: &str,
        expected_size: Option<u64>,
        expected_digest: Option<String>,
    ) -> Result<serde_json::Value, ToolError> {
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
        // The redaction-placeholder guard: build the resulting content in
        // memory and refuse before any byte is written when it would grow
        // the file's `[redacted]` count past what is already on disk — the
        // model must not be able to copy a masked tool result back over the
        // real secret it was standing in for.
        let resulting = splice_text(current, start as usize..end as usize, new_text);
        redaction_guard::refuse_marker_growth(
            rel,
            redaction_guard::count_markers(&file.bytes),
            resulting.as_bytes(),
            ToolError::WorkspaceEdit,
        )?;
        commit_splice(workspace, rel, start..end, size, new_text.as_bytes())?;
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
