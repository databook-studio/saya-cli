use saya_agent::ToolError;

use super::workspace_edit::{WORKSPACE_EDIT_MAX_BYTES, WORKSPACE_EDIT_MAX_FILE_BYTES};
use super::workspace_edit_anchor::hex_digest;
use super::workspace_edit_shared::{commit_splice, workspace_probe, write_new};
use super::{DatabaseTools, redaction_guard};

impl DatabaseTools {
    /// The `append` half of [`DatabaseTools::workspace_edit`]: an empty
    /// range at EOF with `offset` as the positional precondition, committed
    /// through the same [`Workspace::patch_range`] the replace half uses.
    /// Offset 0 on an absent path creates the file; any mismatch refuses,
    /// writes nothing, and reports the current size and digest. A supplied
    /// `expected_size` is a second precondition checked like the replace
    /// half's: a file that moved under the model refuses before the offset
    /// check, so a stated guard never silently passes.
    pub(super) async fn workspace_append(
        &self,
        rel: &str,
        offset: u64,
        chunk: &str,
        expected_size: Option<u64>,
        expected_digest: Option<String>,
    ) -> Result<serde_json::Value, ToolError> {
        if chunk.len() > WORKSPACE_EDIT_MAX_BYTES {
            return Err(ToolError::WorkspaceEditTooLarge {
                path: rel.to_string(),
                limit: WORKSPACE_EDIT_MAX_BYTES,
                found: chunk.len(),
            });
        }
        let Some(workspace) = &self.workspace else {
            return Err(ToolError::WorkspaceUnavailable);
        };
        let target_size = match workspace_probe(workspace, rel)? {
            None => {
                // Absent path: only offset 0 creates. Anything else is a
                // mismatch against the (empty, absent) current state. A
                // supplied `expected_size` guards the same way: anything but
                // the absent size 0 means the file moved under the model.
                if let Some(want) = expected_size
                    && want != 0
                {
                    return Err(ToolError::WorkspaceEditMoved {
                        path: rel.to_string(),
                        expected_size: Some(want),
                        current_size: 0,
                        expected_digest: expected_digest.clone(),
                        current_digest: hex_digest(&[]),
                    });
                }
                if offset != 0 {
                    return Err(ToolError::WorkspaceAppendOffset {
                        path: rel.to_string(),
                        expected_offset: offset,
                        current_size: 0,
                        current_digest: hex_digest(&[]),
                    });
                }
                // The redaction-placeholder guard: a new file starts at 0
                // markers, so any `[redacted]` in the first chunk is a
                // growth from 0 and is refused the same as an existing file.
                redaction_guard::refuse_marker_growth(
                    rel,
                    0,
                    chunk.as_bytes(),
                    ToolError::WorkspaceEdit,
                )?;
                write_new(workspace, rel, chunk.as_bytes())?;
                let after = workspace
                    .read(rel, WORKSPACE_EDIT_MAX_FILE_BYTES)
                    .map_err(|error| ToolError::WorkspaceEdit(error.to_string()))?;
                return Ok(serde_json::json!({
                    "path": rel,
                    "size": after.size,
                    "digest": hex_digest(&after.bytes),
                }));
            }
            Some(size) => size,
        };
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
        if std::str::from_utf8(&file.bytes).is_err() {
            return Err(ToolError::WorkspaceEditNotText {
                path: rel.to_string(),
            });
        }
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
                expected_digest,
                current_digest: digest,
            });
        }
        if offset != size {
            return Err(ToolError::WorkspaceAppendOffset {
                path: rel.to_string(),
                expected_offset: offset,
                current_size: size,
                current_digest: digest,
            });
        }
        // The redaction-placeholder guard: the resulting content is the
        // current file plus `chunk` (an append only ever adds bytes at EOF).
        // Concatenated rather than counted separately per half, so a marker
        // split across the old EOF and the new chunk's start is still seen
        // whole.
        redaction_guard::refuse_marker_growth(
            rel,
            redaction_guard::count_markers(&file.bytes),
            &[file.bytes.as_slice(), chunk.as_bytes()].concat(),
            ToolError::WorkspaceEdit,
        )?;
        commit_splice(workspace, rel, size..size, size, chunk.as_bytes())?;
        let after = workspace
            .read(rel, WORKSPACE_EDIT_MAX_FILE_BYTES)
            .map_err(|error| ToolError::WorkspaceEdit(error.to_string()))?;
        Ok(serde_json::json!({
            "path": rel,
            "size": after.size,
            "digest": hex_digest(&after.bytes),
        }))
    }
}
