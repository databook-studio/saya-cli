use saya_agent::{CancellationToken, ToolError};
use saya_types::EditSource;

use super::super::super::DatabaseTools;
use super::super::checkpoint::{Capture, RefusalClass, classify_edit};
use super::super::excerpt::changed_excerpt;
use super::super::redaction_guard;
use super::anchor::{find_overlapping_matches, hex_digest, match_lines};
use super::args::{MAX_MULTI_EDIT_TEXT_BYTES, MAX_MULTI_EDITS};
use super::shared::{map_commit_error, splice_all};
use super::{WORKSPACE_EDIT_MAX_BYTES, WORKSPACE_EDIT_MAX_FILE_BYTES};

#[cfg(test)]
#[path = "multi_tests.rs"]
mod tests;

impl DatabaseTools {
    pub(super) async fn workspace_multi_edit(
        &self,
        rel: &str,
        edits: &[(String, String)],
        expected_size: Option<u64>,
        expected_digest: Option<&str>,
        cancellation: &CancellationToken,
    ) -> Result<serde_json::Value, ToolError> {
        if edits.is_empty() || edits.len() > MAX_MULTI_EDITS {
            return Err(ToolError::UnsupportedProperty);
        }
        let text_bytes = edits.iter().try_fold(0usize, |total, (old, new)| {
            total.checked_add(old.len())?.checked_add(new.len())
        });
        if text_bytes.is_none_or(|size| size > MAX_MULTI_EDIT_TEXT_BYTES)
            || edits.iter().any(|(old, new)| {
                old.is_empty()
                    || old.len() > WORKSPACE_EDIT_MAX_BYTES
                    || new.len() > WORKSPACE_EDIT_MAX_BYTES
            })
        {
            return Err(ToolError::WorkspaceEditTooLarge {
                path: rel.to_owned(),
                limit: MAX_MULTI_EDIT_TEXT_BYTES,
                found: text_bytes.unwrap_or(usize::MAX),
            });
        }
        let Some(workspace) = &self.workspace else {
            return Err(ToolError::WorkspaceUnavailable);
        };
        let file = workspace
            .read(rel, WORKSPACE_EDIT_MAX_FILE_BYTES)
            .map_err(|error| ToolError::WorkspaceEdit(error.to_string()))?;
        if file.size > WORKSPACE_EDIT_MAX_FILE_BYTES {
            return Err(ToolError::WorkspaceEdit(
                saya_harness::HarnessError::BoundsExceeded {
                    path: rel.to_owned(),
                    found: file.size,
                    max: WORKSPACE_EDIT_MAX_FILE_BYTES,
                }
                .to_string(),
            ));
        }
        let current =
            std::str::from_utf8(&file.bytes).map_err(|_| ToolError::WorkspaceEditNotText {
                path: rel.to_owned(),
            })?;
        let current_digest = hex_digest(&file.bytes);
        // A batch with no digest at all is a read-first refusal, raised
        // before the size check and carrying the state the model retries
        // from.
        let Some(expected_digest) = expected_digest else {
            return Err(ToolError::WorkspaceDigestRequired {
                operation: "edit",
                path: rel.to_owned(),
                size: file.size,
                current_digest,
            });
        };
        // A stale digest is re-anchored only when this run's own earlier
        // edit/write of this path is the only reason the file moved; a
        // change by anyone else still refuses. Every anchor below is
        // resolved against the current content and must match exactly once.
        let rebased = expected_digest != current_digest
            && self.read_witnesses.accepts_stale_digest(
                rel,
                workspace,
                expected_digest,
                &current_digest,
            );
        if !rebased && expected_size.is_some_and(|size| size != file.size) {
            self.read_witnesses
                .invalidate_if_stale(rel, workspace, &current_digest, file.size);
            return Err(ToolError::WorkspaceEditMoved {
                path: rel.to_owned(),
                expected_size,
                current_size: file.size,
                expected_digest: Some(expected_digest.to_owned()),
                current_digest,
            });
        }
        if !rebased && expected_digest != current_digest {
            self.read_witnesses
                .invalidate_if_stale(rel, workspace, &current_digest, file.size);
            return Err(ToolError::WorkspaceEditMoved {
                path: rel.to_owned(),
                expected_size,
                current_size: file.size,
                expected_digest: Some(expected_digest.to_owned()),
                current_digest,
            });
        }
        // The digest the commit re-verifies: the current state when the
        // stale digest was re-anchored, the model's own observation
        // otherwise.
        let observed_digest = if rebased {
            current_digest.clone()
        } else {
            expected_digest.to_owned()
        };

        let mut replacements = Vec::with_capacity(edits.len());
        let mut bytes_replaced = 0usize;
        let mut bytes_written = 0usize;
        for (old_text, new_text) in edits {
            let matches = find_overlapping_matches(current, old_text);
            if matches.is_empty() {
                return Err(ToolError::WorkspaceEditNoMatch {
                    path: rel.to_owned(),
                    matches: 0,
                    size: file.size,
                    digest: current_digest,
                    hint: String::new(),
                });
            }
            if matches.len() != 1 {
                return Err(ToolError::WorkspaceEditAmbiguous {
                    path: rel.to_owned(),
                    matches: matches.len(),
                    lines: match_lines(current, &matches),
                    size: file.size,
                    digest: current_digest,
                });
            }
            let start = matches[0];
            let end = start + old_text.len();
            bytes_replaced += old_text.len();
            bytes_written += new_text.len();
            replacements.push((start..end, new_text.as_bytes()));
        }
        replacements.sort_by_key(|(range, _)| range.start);
        if replacements
            .windows(2)
            .any(|pair| pair[0].0.end > pair[1].0.start)
        {
            return Err(ToolError::WorkspaceEdit(
                super::MULTI_EDIT_OVERLAP_REFUSAL.into(),
            ));
        }
        let final_size =
            replacements
                .iter()
                .try_fold(current.len(), |size, (range, replacement)| {
                    size.checked_sub(range.end - range.start)?
                        .checked_add(replacement.len())
                });
        if final_size.is_none_or(|size| size as u64 > WORKSPACE_EDIT_MAX_FILE_BYTES) {
            return Err(ToolError::WorkspaceEditTooLarge {
                path: rel.to_owned(),
                limit: WORKSPACE_EDIT_MAX_FILE_BYTES as usize,
                found: final_size.unwrap_or(usize::MAX),
            });
        }
        let resulting = splice_all(&file.bytes, &replacements);
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
        if cancellation.is_cancelled() {
            if let (Some(recorder), Some(Capture::Recorded(pending))) = (recorder, capture.as_ref())
            {
                recorder.settle(pending, RefusalClass::Clean);
            }
            return Err(ToolError::NoEffect(Box::new(ToolError::Runner(
                "workspace edit was cancelled before publication".into(),
            ))));
        }
        if let Err(error) = workspace.write_guarded(rel, &observed_digest, &resulting) {
            // A failed batch edit leaves the witness alone.
            let error = map_commit_error(workspace, rel, &observed_digest, file.size, error);
            if let (Some(recorder), Some(Capture::Recorded(pending))) = (recorder, capture.as_ref())
            {
                recorder.settle(pending, classify_edit(&error));
            }
            return Err(error);
        }
        let digest = hex_digest(&resulting);
        let size = resulting.len() as u64;
        self.read_witnesses.remember_mutation(
            rel,
            workspace,
            Some(&current_digest),
            digest.clone(),
            size,
        );
        let mut result = serde_json::json!({
            "path": rel,
            "bytes_replaced": bytes_replaced,
            "bytes_written": bytes_written,
            "size": size,
            "digest": digest.clone(),
        });
        if let Ok(text) = std::str::from_utf8(&resulting)
            && let Some(excerpt) = changed_excerpt(text, &result_ranges(&replacements))
        {
            result["changed_excerpt"] = serde_json::Value::String(excerpt);
        }
        if let Some(recorder) = recorder
            && let Some(label) = recorder.finish_label(capture.as_ref(), &digest, size)
        {
            result["recovery"] = serde_json::Value::String(label);
        }
        Ok(result)
    }
}

/// The changed byte ranges in the published bytes: each replacement shifted
/// by the size deltas of the replacements before it.
fn result_ranges(replacements: &[(std::ops::Range<usize>, &[u8])]) -> Vec<std::ops::Range<usize>> {
    let mut ranges = Vec::with_capacity(replacements.len());
    let mut delta = 0isize;
    for (range, replacement) in replacements {
        let start = (range.start as isize + delta) as usize;
        ranges.push(start..start + replacement.len());
        delta += replacement.len() as isize - (range.end - range.start) as isize;
    }
    ranges
}
