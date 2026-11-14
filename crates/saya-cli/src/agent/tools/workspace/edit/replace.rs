use saya_agent::ToolError;
use saya_types::EditSource;

use super::super::super::DatabaseTools;
use super::super::checkpoint::{Capture, classify_edit};
use super::super::excerpt::changed_excerpt;
use super::super::redaction_guard;
use super::anchor::{find_matches, hex_digest, match_lines};
use super::shared::{commit_splice, splice_text, workspace_size};
use super::{WORKSPACE_EDIT_MAX_BYTES, WORKSPACE_EDIT_MAX_FILE_BYTES};

/// The result note a lenient re-anchor adds: the model's digest was stale, the
/// anchor still resolved once on the current content, and the edit applied.
const LENIENT_NOTE: &str =
    "the file changed since you read it; the edit applied to the current content";

impl DatabaseTools {
    /// The `replace` half of [`DatabaseTools::workspace_edit`]: resolve
    /// `old_text` to exactly one byte range, verify the observed digest,
    /// then commit the splice through the guarded
    /// `Workspace::patch_range_guarded` — so a same-size change between
    /// this read and the commit is refused too. A replace with no digest at
    /// all refuses with the file's current size and digest, read here so the
    /// model retries without a separate read. With `replace_all` the
    /// same digest checks hold, and the anchor resolves to every occurrence
    /// instead of exactly one.
    pub(super) async fn workspace_replace(
        &self,
        rel: &str,
        old_text: &str,
        new_text: &str,
        replace_all: bool,
        expected_size: Option<u64>,
        expected_digest: Option<&str>,
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
        // A replace with no digest at all is a read-first refusal, and it is
        // raised before the `expected_size` check: the file has been read by
        // now, so the refusal carries the state the model retries from.
        let Some(expected_digest) = expected_digest else {
            return Err(ToolError::WorkspaceDigestRequired {
                operation: "edit",
                path: rel.to_string(),
                size,
                current_digest: digest,
            });
        };
        // A stale digest this run's own earlier write or edit explains is
        // re-anchored as before. A stale digest it does not explain — the file
        // moved under the model — is re-anchored too, but only when the anchor
        // still resolves under the replace's own count rule: exactly once, or
        // at least once with `replace_all`. Zero or several matches keep
        // today's moved-anchor refusal. `expected_size` alone never refuses
        // once the digest and the anchor agree.
        let stale = expected_digest != digest;
        let rebased = stale
            && self
                .read_witnesses
                .accepts_stale_digest(rel, workspace, expected_digest, &digest);
        let lenient = if stale && !rebased {
            let hits = find_matches(current, old_text);
            let resolvable = if replace_all {
                !hits.is_empty()
            } else {
                hits.len() == 1
            };
            if !resolvable {
                self.read_witnesses
                    .invalidate_if_stale(rel, workspace, &digest, size);
                return Err(ToolError::WorkspaceEditMoved {
                    path: rel.to_string(),
                    expected_size,
                    current_size: size,
                    expected_digest: Some(expected_digest.to_string()),
                    current_digest: digest,
                });
            }
            true
        } else {
            false
        };
        // The digest the commit re-verifies: the current state whenever the
        // model's observation was stale (own chain or a lenient re-anchor),
        // the model's own observation otherwise. The commit is guarded, so a
        // change between this resolution and the write still refuses.
        let observed_digest = if rebased || lenient {
            digest.as_str()
        } else {
            expected_digest
        };
        let note = lenient.then_some(LENIENT_NOTE);
        // `replace_all` is the whole-file form: it re-resolves the anchor
        // against the snapshot read above and publishes every occurrence as
        // one commit (see `replace_all.rs`), so it branches before the
        // exactly-once counting below — which is what the default keeps.
        if replace_all {
            let mut result = self.workspace_replace_all(
                rel,
                workspace,
                &file,
                old_text,
                new_text,
                observed_digest,
            )?;
            if let Some(note) = note {
                result["note"] = serde_json::Value::String(note.into());
            }
            return Ok(result);
        }
        let hits = find_matches(current, old_text);
        if hits.is_empty() {
            return Err(ToolError::WorkspaceEditNoMatch {
                path: rel.to_string(),
                matches: 0,
                size,
                digest,
                hint: super::hint::nearest_hint(current, old_text),
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
        #[cfg(test)]
        if let Some(hook) = &self.before_commit {
            hook();
        }
        if let Err(error) = commit_splice(
            workspace,
            rel,
            start..end,
            size,
            Some(observed_digest),
            new_text.as_bytes(),
        ) {
            // A failed edit leaves the witness alone — the next attempt's
            // digest comparison decides whether the observation still holds.
            if let (Some(recorder), Some(Capture::Recorded(pending))) = (recorder, capture.as_ref())
            {
                recorder.settle(pending, classify_edit(&error));
            }
            return Err(error);
        }
        let resulting_digest = hex_digest(resulting.as_bytes());
        let resulting_size = resulting.len() as u64;
        // The witness chains what landed onto the content it applied
        // against, so a batched follow-up carrying this file's pre-edit
        // digest can be re-anchored.
        self.read_witnesses.remember_mutation(
            rel,
            workspace,
            Some(&digest),
            resulting_digest.clone(),
            resulting_size,
        );
        let digest = resulting_digest;
        let size = resulting_size;
        let mut result = serde_json::json!({
            "path": rel,
            "bytes_replaced": old_text.len(),
            "bytes_written": new_text.len(),
            "replacements": 1,
            "size": size,
            "digest": digest.clone(),
        });
        if let Some(note) = note {
            result["note"] = serde_json::Value::String(note.into());
        }
        let changed_start = start as usize;
        if let Some(excerpt) = changed_excerpt(
            &resulting,
            std::slice::from_ref(&(changed_start..changed_start + new_text.len())),
        ) {
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
