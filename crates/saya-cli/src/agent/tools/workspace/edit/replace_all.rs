//! The `replace_all` half of the replace variant: every non-overlapping
//! occurrence of the one anchor is spliced against the same original
//! snapshot and the whole result is published once — one atomic commit, one
//! checkpoint, one digest — so a rename across one file is one call.
//! `replace_all: false`, the default, never reaches this module: it keeps the
//! exactly-once refusal in `replace.rs`. Reached only after that module has
//! read the target, refused a non-UTF-8 one, and verified the observed
//! `expected_size`/`expected_digest`, so the checks here are the ones the
//! many-site form adds: at least one occurrence, the resulting size bound,
//! and the redaction-placeholder guard.

use saya_agent::ToolError;
use saya_harness::workspace::{ReadFile, Workspace};
use saya_types::EditSource;

use super::super::super::DatabaseTools;
use super::super::checkpoint::{Capture, classify_edit};
use super::super::excerpt::changed_excerpt;
use super::super::redaction_guard;
use super::WORKSPACE_EDIT_MAX_FILE_BYTES;
use super::anchor::{find_matches, hex_digest};
use super::shared::{map_commit_error, splice_all};

impl DatabaseTools {
    /// Replaces every occurrence of `old_text` in `file`'s bytes with
    /// `new_text` and commits the result as one whole-file replace behind the
    /// harness's guarded fresh-read check. `find_matches` reports ascending,
    /// non-overlapping offsets, so the ranges never need the batch form's
    /// overlap check; the result is bounded and screened before any byte
    /// lands, and the reported `replacements` count is the number of sites
    /// the commit actually changed.
    pub(super) fn workspace_replace_all(
        &self,
        rel: &str,
        workspace: &Workspace,
        file: &ReadFile,
        old_text: &str,
        new_text: &str,
        expected_digest: &str,
    ) -> Result<serde_json::Value, ToolError> {
        let current =
            std::str::from_utf8(&file.bytes).map_err(|_| ToolError::WorkspaceEditNotText {
                path: rel.to_owned(),
            })?;
        let hits = find_matches(current, old_text);
        if hits.is_empty() {
            return Err(ToolError::WorkspaceEditNoMatch {
                path: rel.to_owned(),
                matches: 0,
                size: file.size,
                digest: hex_digest(&file.bytes),
                hint: super::hint::nearest_hint(current, old_text),
            });
        }
        let replacements: Vec<(std::ops::Range<usize>, &[u8])> = hits
            .iter()
            .map(|start| (*start..start + old_text.len(), new_text.as_bytes()))
            .collect();
        let resulting = splice_all(&file.bytes, &replacements);
        if resulting.len() as u64 > WORKSPACE_EDIT_MAX_FILE_BYTES {
            return Err(ToolError::WorkspaceEditTooLarge {
                path: rel.to_owned(),
                limit: WORKSPACE_EDIT_MAX_FILE_BYTES as usize,
                found: resulting.len(),
            });
        }
        // The redaction-placeholder guard, exactly as the single-site form
        // applies it: a replacement may not grow the file's `[redacted]`
        // count past what is already on disk.
        redaction_guard::refuse_marker_growth(
            rel,
            redaction_guard::count_markers(&file.bytes),
            &resulting,
            ToolError::WorkspaceEdit,
        )?;
        let recorder = self
            .checkpoints
            .as_ref()
            .filter(|recorder| recorder.active());
        let capture = recorder
            .map(|recorder| recorder.begin(rel, EditSource::WorkspaceEdit, Some(&file.bytes)));
        if let Some(text) = capture.as_ref().and_then(Capture::refused_text) {
            return Err(ToolError::WorkspaceEdit(text.to_owned()));
        }
        if let Err(error) = workspace.write_guarded(rel, expected_digest, &resulting) {
            // A failed edit leaves the witness alone — the next attempt's
            // digest comparison decides whether the observation still holds.
            let error = map_commit_error(workspace, rel, expected_digest, file.size, error);
            if let (Some(recorder), Some(Capture::Recorded(pending))) = (recorder, capture.as_ref())
            {
                recorder.settle(pending, classify_edit(&error));
            }
            return Err(error);
        }
        let digest = hex_digest(&resulting);
        let size = resulting.len() as u64;
        // Chain the landed state onto the snapshot it applied against, so a
        // batched follow-up with the file's pre-edit digest re-anchors.
        self.read_witnesses.remember_mutation(
            rel,
            workspace,
            Some(&hex_digest(&file.bytes)),
            digest.clone(),
            size,
        );
        let mut result = serde_json::json!({
            "path": rel,
            "bytes_replaced": hits.len() * old_text.len(),
            "bytes_written": hits.len() * new_text.len(),
            "replacements": hits.len(),
            "size": size,
            "digest": digest.clone(),
        });
        if let Ok(text) = std::str::from_utf8(&resulting) {
            let mut changed = Vec::with_capacity(hits.len());
            let mut delta = 0isize;
            for start in &hits {
                let shifted = (*start as isize + delta) as usize;
                changed.push(shifted..shifted + new_text.len());
                delta += new_text.len() as isize - old_text.len() as isize;
            }
            if let Some(excerpt) = changed_excerpt(text, &changed) {
                result["changed_excerpt"] = serde_json::Value::String(excerpt);
            }
        }
        if let Some(recorder) = recorder
            && let Some(label) = recorder.finish_label(capture.as_ref(), &digest, size)
        {
            result["recovery"] = serde_json::Value::String(label);
        }
        Ok(result)
    }
}
