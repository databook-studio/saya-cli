use saya_agent::ToolError;
use saya_types::EditSource;

use super::super::super::DatabaseTools;
use super::super::checkpoint::{Capture, classify_edit};
use super::super::redaction_guard;
use super::anchor::hex_digest;
use super::args::APPEND_REQUIRES_DIGEST;
use super::shared::{commit_splice, workspace_probe, write_new};
use super::{WORKSPACE_EDIT_MAX_BYTES, WORKSPACE_EDIT_MAX_FILE_BYTES};

impl DatabaseTools {
    /// The `append` half of [`DatabaseTools::workspace_edit`]: an empty
    /// range at EOF with `offset` as the positional precondition, committed
    /// through the guarded `Workspace::patch_range_guarded` the replace
    /// half uses — the digest the model observed is re-verified at the
    /// commit's own fresh read, so a same-size change between this read
    /// and the commit is refused too. Appending to an existing file
    /// requires the observed `expected_digest`, refused read-first before
    /// anything read-derived leaves the function; offset 0 on an absent
    /// path creates the file through `Workspace::create_new`. Any mismatch
    /// refuses, writes nothing, and reports the current size and digest. A
    /// supplied `expected_size` is a second precondition checked like the
    /// replace half's: a file that moved under the model refuses before
    /// the offset check, so a stated guard never silently passes.
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
        let probed = workspace_probe(workspace, rel)?;
        // The existing-file branch's first line: the digest precondition is
        // raised before anything read-derived leaves the function (so the
        // message cannot leak the file's current digest) and before the
        // offset check (whose payload does carry it) — a missing digest is
        // a read-first refusal, never an offset refusal. The absent branch
        // stays digest-free: offset 0 there creates the file.
        if probed.is_some() && expected_digest.is_none() {
            return Err(ToolError::WorkspaceEdit(APPEND_REQUIRES_DIGEST.to_owned()));
        }
        let target_size = match probed {
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
                // The create's capture is prepared before the write: no before-image
                // exists (the file is absent), so only the path/source are
                // recorded.
                let recorder = self
                    .checkpoints
                    .as_ref()
                    .filter(|recorder| recorder.active());
                let capture =
                    recorder.map(|recorder| recorder.begin(rel, EditSource::WorkspaceEdit, None));
                if let Some(text) = capture.as_ref().and_then(Capture::refused_text) {
                    return Err(ToolError::WorkspaceEdit(text.to_owned()));
                }
                if let Err(error) = write_new(workspace, rel, chunk.as_bytes()) {
                    if let (Some(recorder), Some(Capture::Recorded(pending))) =
                        (recorder, capture.as_ref())
                    {
                        recorder.settle(pending, classify_edit(&error));
                    }
                    return Err(error);
                }
                let after = workspace
                    .read(rel, WORKSPACE_EDIT_MAX_FILE_BYTES)
                    .map_err(|error| ToolError::WorkspaceEdit(error.to_string()))?;
                let mut result = serde_json::json!({
                    "path": rel,
                    "size": after.size,
                    "digest": hex_digest(&after.bytes),
                });
                if let Some(recorder) = recorder
                    && let Some(label) = recorder.finish_label(
                        capture.as_ref(),
                        &hex_digest(&after.bytes),
                        after.size,
                    )
                {
                    result["recovery"] = serde_json::Value::String(label);
                }
                return Ok(result);
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
        // The observed digest anchors the commit: a same-size change
        // between the read above and the commit's own fresh re-read is
        // refused as the typed moved error. `expected_digest` is Some by
        // the precondition check above (the absent branch never reaches
        // this line).
        // Checkpoint capture before the guarded commit, with the existing
        // file's exact bytes.
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
            size..size,
            size,
            expected_digest.as_deref(),
            chunk.as_bytes(),
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
