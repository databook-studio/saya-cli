use saya_agent::ToolError;
use saya_types::EditSource;

use super::super::super::DatabaseTools;
use super::super::checkpoint::{Capture, classify_edit};
use super::super::redaction_guard;
use super::anchor::{find_matches, hex_digest, match_lines};
use super::shared::{commit_splice, splice_text, workspace_size};
use super::{WORKSPACE_EDIT_MAX_BYTES, WORKSPACE_EDIT_MAX_FILE_BYTES};

impl DatabaseTools {
    /// The `replace` half of [`DatabaseTools::workspace_edit`]: resolve
    /// `old_text` to exactly one byte range, verify the observed digest,
    /// then commit the splice through the guarded
    /// `Workspace::patch_range_guarded` — so a same-size change between
    /// this read and the commit is refused too.
    pub(super) async fn workspace_replace(
        &self,
        rel: &str,
        old_text: &str,
        new_text: &str,
        expected_size: Option<u64>,
        expected_digest: &str,
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
                expected_digest: Some(expected_digest.to_string()),
                current_digest: digest,
            });
        }
        if expected_digest != digest {
            return Err(ToolError::WorkspaceEditMoved {
                path: rel.to_string(),
                expected_size,
                current_size: size,
                expected_digest: Some(expected_digest.to_string()),
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
        // Checkpoint capture happens before the guarded commit: a capture
        // refusal prevents the edit, and a refused commit settles the
        // record (clean refusal drops it, an ambiguous one stays
        // `Uncertain`). The whole-file before-image is exactly what the
        // size-bound read above produced.
        let recorder = self
            .checkpoints
            .as_ref()
            .filter(|recorder| recorder.active());
        let capture = recorder
            .map(|recorder| recorder.begin(rel, EditSource::WorkspaceEdit, Some(&file.bytes)));
        if let Some(text) = capture.as_ref().and_then(Capture::refused_text) {
            return Err(ToolError::WorkspaceEdit(text.to_owned()));
        }
        let after = match commit_splice(
            workspace,
            rel,
            start..end,
            size,
            Some(expected_digest),
            new_text.as_bytes(),
        ) {
            Ok(()) => workspace
                .read(rel, WORKSPACE_EDIT_MAX_FILE_BYTES)
                .map_err(|error| ToolError::WorkspaceEdit(error.to_string()))?,
            Err(error) => {
                if let (Some(recorder), Some(Capture::Recorded(pending))) =
                    (recorder, capture.as_ref())
                {
                    recorder.settle(pending, classify_edit(&error));
                }
                return Err(error);
            }
        };
        let mut result = serde_json::json!({
            "path": rel,
            "bytes_replaced": old_text.len(),
            "bytes_written": new_text.len(),
            "size": after.size,
            "digest": hex_digest(&after.bytes),
        });
        if let Some(recorder) = recorder
            && let Some(label) =
                recorder.finish_label(capture.as_ref(), &hex_digest(&after.bytes), after.size)
        {
            result["recovery"] = serde_json::Value::String(label);
        }
        Ok(result)
    }
}
