//! `notebook_edit`: one cell of a notebook replaced, inserted or deleted.
//!
//! The write rules are `edit`'s, unchanged — the same contained workspace,
//! the same `expected_digest` precondition (or the witness of an earlier read
//! in this run), the same checkpoint capture, the same redaction-placeholder
//! guard, the same guarded commit behind a fresh read, and the same typed
//! refusals carrying counts, sizes and digests — never file content.
//!
//! What is its own is that the notebook is spliced, not re-serialized: the
//! edit writes over one span of the file's own bytes (see [`super::splice`]),
//! and the whole result is re-parsed before it is committed, so a splice that
//! somehow produced invalid JSON would be refused rather than written.

use saya_agent::ToolError;
use saya_types::EditSource;
use serde_json::Value;

use super::super::super::DatabaseTools;
use super::super::checkpoint::{Capture, classify_edit};
use super::super::edit::{
    WORKSPACE_EDIT_MAX_BYTES, WORKSPACE_EDIT_MAX_FILE_BYTES, commit_splice, hex_digest,
    workspace_size,
};
use super::super::redaction_guard;
use super::args::{self, Request};
use super::scan;
use super::splice;

impl DatabaseTools {
    /// Changes one cell of the contained notebook `path`. See the module
    /// comment for the write rules this shares with `edit`.
    pub(in crate::agent::tools) async fn workspace_notebook_edit(
        &self,
        arguments: &Value,
    ) -> Result<Value, ToolError> {
        // The inner half records the mutation chain at the point where it
        // knows the content the splice applied against.
        self.notebook_edit_inner(arguments)
    }

    fn notebook_edit_inner(&self, arguments: &Value) -> Result<Value, ToolError> {
        let request = args::parse_arguments(arguments)?;
        let path = request.path();
        // The source bound is the `edit` bound, and it is checked before the
        // file is touched: an over-bound source is refused whole.
        if let Some(source) = source_of(&request)
            && source.len() > WORKSPACE_EDIT_MAX_BYTES
        {
            return Err(ToolError::WorkspaceEditTooLarge {
                path: path.to_owned(),
                limit: WORKSPACE_EDIT_MAX_BYTES,
                found: source.len(),
            });
        }
        let Some(workspace) = &self.workspace else {
            return Err(ToolError::WorkspaceUnavailable);
        };
        let target_size = workspace_size(workspace, path)?;
        if target_size > WORKSPACE_EDIT_MAX_FILE_BYTES {
            return Err(ToolError::WorkspaceEdit(
                saya_harness::HarnessError::BoundsExceeded {
                    path: path.to_owned(),
                    found: target_size,
                    max: WORKSPACE_EDIT_MAX_FILE_BYTES,
                }
                .to_string(),
            ));
        }
        let file = workspace
            .read(path, WORKSPACE_EDIT_MAX_FILE_BYTES)
            .map_err(|error| ToolError::WorkspaceEdit(error.to_string()))?;
        if std::str::from_utf8(&file.bytes).is_err() {
            return Err(ToolError::WorkspaceEditNotText {
                path: path.to_owned(),
            });
        }
        let digest = hex_digest(&file.bytes);
        // A call with no digest at all is the read-first refusal, raised after
        // the file was read so it carries the state the model retries from.
        let Some(expected) = request.expected_digest() else {
            return Err(ToolError::WorkspaceDigestRequired {
                operation: "notebook_edit",
                path: path.to_owned(),
                size: file.size,
                current_digest: digest,
            });
        };
        if expected != digest {
            self.read_witnesses
                .invalidate_if_stale(path, workspace, &digest, file.size);
            return Err(ToolError::WorkspaceEditMoved {
                path: path.to_owned(),
                expected_size: None,
                current_size: file.size,
                expected_digest: Some(expected.to_owned()),
                current_digest: digest,
            });
        }
        let cells = scan::scan(&file.bytes).ok_or_else(|| splice::not_a_notebook(path))?;
        let planned = splice::plan(&request, &file.bytes, &cells, path)?;
        let resulting = [
            &file.bytes[..planned.range.start],
            planned.bytes.as_slice(),
            &file.bytes[planned.range.end..],
        ]
        .concat();
        if resulting.len() as u64 > WORKSPACE_EDIT_MAX_FILE_BYTES {
            return Err(ToolError::WorkspaceEditTooLarge {
                path: path.to_owned(),
                limit: WORKSPACE_EDIT_MAX_FILE_BYTES as usize,
                found: resulting.len(),
            });
        }
        // The notebook this writes back is valid JSON, or it is not written.
        // The splice is built to be, so this only ever catches a fault in it.
        if serde_json::from_slice::<Value>(&resulting).is_err() {
            return Err(ToolError::WorkspaceEdit(format!(
                "refused: the edit would leave {path} as invalid JSON, and the file was not changed"
            )));
        }
        redaction_guard::refuse_marker_growth(
            path,
            redaction_guard::count_markers(&file.bytes),
            &resulting,
            ToolError::WorkspaceEdit,
        )?;
        // Capture happens before the commit, exactly as `edit` does it: a
        // capture refusal prevents the edit, and a refused commit settles the
        // record.
        let recorder = self
            .checkpoints
            .as_ref()
            .filter(|recorder| recorder.active());
        let capture = recorder
            .map(|recorder| recorder.begin(path, EditSource::NotebookEdit, Some(&file.bytes)));
        if let Some(text) = capture.as_ref().and_then(Capture::refused_text) {
            return Err(ToolError::WorkspaceEdit(text.to_owned()));
        }
        if let Err(error) = commit_splice(
            workspace,
            path,
            planned.range.start as u64..planned.range.end as u64,
            file.size,
            Some(expected),
            &planned.bytes,
        ) {
            if let (Some(recorder), Some(Capture::Recorded(pending))) = (recorder, capture.as_ref())
            {
                recorder.settle(pending, classify_edit(&error));
            }
            return Err(error);
        }
        let resulting_digest = hex_digest(&resulting);
        let resulting_size = resulting.len() as u64;
        self.read_witnesses.remember_mutation(
            path,
            workspace,
            Some(&digest),
            resulting_digest.clone(),
            resulting_size,
        );
        let digest = resulting_digest;
        let size = resulting_size;
        let mut result = serde_json::json!({
            "path": path,
            "mode": request.mode(),
            "cell": planned.cell,
            "cells": planned.cells,
            "size": size,
            "digest": digest.clone(),
        });
        if let Some(recorder) = recorder
            && let Some(label) = recorder.finish_label(capture.as_ref(), &digest, size)
        {
            result["recovery"] = Value::String(label);
        }
        Ok(result)
    }
}

/// The source a request writes, for the byte bound. A delete writes none.
fn source_of(request: &Request) -> Option<&str> {
    match request {
        Request::Replace { source, .. } | Request::Insert { source, .. } => Some(source),
        Request::Delete { .. } => None,
    }
}
