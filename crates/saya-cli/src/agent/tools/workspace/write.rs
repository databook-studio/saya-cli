//! The one file-writing tool: an atomic, bounded, create-if-absent write
//! into the run workspace. Path validation, symlink refusal, the prefix
//! check, and the temp-then-rename publication are the harness's (`saya_
//! harness::workspace::Workspace::create_new` and `::write_guarded`) — never
//! re-derived here. This tool's own jobs are argument typing, the byte
//! bound, the `expected_digest` precondition, and surfacing the harness's
//! containment reason as the error text.

use saya_agent::ToolError;
use saya_harness::{HarnessError, workspace::Workspace};
use saya_types::EditSource;

use super::super::DatabaseTools;
use super::checkpoint::{Capture, classify_write};
use super::edit::WORKSPACE_EDIT_MAX_FILE_BYTES;
use super::edit::hex_digest;
use super::excerpt::head_excerpt;
use super::redaction_guard;

/// The bound on a workspace write's `content`: 1 MiB. A whole-file write
/// of that size is one call rather than a create plus a chain of appends, so
/// the model never has to re-arm a digest and an offset per chunk. It stays
/// far inside every provider's per-message tool-call budget, so a write is
/// never the call the provider silently clips, and it stays below the
/// harness's own 8 MiB write path. Over the bound is a typed refusal — never
/// a truncated write: a partially written file is worse than a refused one,
/// and content past the bound can still be split across several files.
pub(crate) const WORKSPACE_WRITE_MAX_BYTES: usize = 1024 * 1024;

/// The refusal when a digest-carrying write names an absent path: a guarded
/// replace is not a create — omitting `expected_digest` is the create form.
pub(in crate::agent::tools) const MISSING_FILE_NOT_A_REPLACE: &str =
    "the file does not exist; omit expected_digest to create it";

/// The refusal when a no-digest write names an existing entry: the file's
/// current size and digest travel with it so the model retries with the
/// state it was just handed instead of spending a read turn first. A file
/// whose state cannot be read falls back to the plain containment text.
fn digest_required(workspace: &Workspace, rel: &str) -> ToolError {
    match workspace.read(rel, 0) {
        Ok(file) => ToolError::WorkspaceDigestRequired {
            operation: "write",
            path: rel.to_string(),
            size: file.size,
            current_digest: file.digest,
        },
        Err(other) => ToolError::WorkspaceWrite(other.to_string()),
    }
}

/// The refusal when the guarded replace's fresh re-read hashes to a digest
/// the model did not observe: the file moved since it was seen. The current
/// size and digest travel with it so the model retries without a
/// separate read; a state that cannot be re-read falls back to the plain
/// containment text.
fn write_moved(
    workspace: &Workspace,
    rel: &str,
    expected_digest: &str,
    fallback_size: u64,
) -> ToolError {
    match workspace.read(rel, 0) {
        Ok(file) => ToolError::WorkspaceWriteMoved {
            path: rel.to_string(),
            expected_digest: expected_digest.to_owned(),
            current_size: file.size,
            current_digest: file.digest,
        },
        Err(_) => ToolError::WorkspaceWriteMoved {
            path: rel.to_string(),
            expected_digest: expected_digest.to_owned(),
            current_size: fallback_size,
            current_digest: hex_digest(&[]),
        },
    }
}

impl DatabaseTools {
    /// Writes one file into the run workspace under containment:
    /// create-if-absent by default, and a replacement only with an explicit
    /// digest or the read witness of any earlier read of that file in this
    /// run — a stale or same-size-changed file refuses with the file's
    /// current size and digest, so the model is never last-writer-wins and
    /// never needs a second read to retry. Like the workspace read tools this
    /// never touches a
    /// connection — dispatch routes it before connection resolution — and it
    /// records nothing into the knowledge store: a workspace write is not
    /// evidence about a database object. With no workspace attached it
    /// denies with a typed error: no workspace, no write.
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
        // An explicit null counts as absent, as in edit's parser.
        let expected_digest = match arguments.get("expected_digest") {
            None | Some(serde_json::Value::Null) => None,
            Some(value) => Some(
                value
                    .as_str()
                    .ok_or(ToolError::ExpectedDigestNotString)?
                    .to_owned(),
            ),
        };
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
        // Checkpoint capture around the commit: the recorder gates each branch
        // below — a capture refusal prevents the write, a refused commit
        // settles the record (clean refusal drops it, an ambiguous one
        // stays `Uncertain`), and a landed commit is finalized with the
        // digest of the bytes the call carried.
        let recorder = self
            .checkpoints
            .as_ref()
            .filter(|recorder| recorder.active());
        // The content this write applied against, for the run's mutation
        // chain: `None` on the create branch, the re-verified current digest
        // on the replace branch.
        let mut from_digest: Option<String> = None;
        let label = match expected_digest {
            // Create-if-absent: an existing entry refuses whole and is left
            // byte-identical; replacing goes through the guarded branch.
            None => {
                let capture =
                    recorder.map(|recorder| recorder.begin(rel, EditSource::WorkspaceWrite, None));
                if let Some(text) = capture.as_ref().and_then(Capture::refused_text) {
                    return Err(ToolError::WorkspaceWrite(text.to_owned()));
                }
                if let Err(error) = workspace
                    .create_new(rel, content.as_bytes())
                    .map_err(|error| match error {
                        HarnessError::AlreadyExists { .. } => digest_required(workspace, rel),
                        other => ToolError::WorkspaceWrite(other.to_string()),
                    })
                {
                    if let (Some(recorder), Some(Capture::Recorded(pending))) =
                        (recorder, capture.as_ref())
                    {
                        recorder.settle(pending, classify_write(&error));
                    }
                    // A failed write leaves the witness alone — a refusal that
                    // changed nothing leaves the model's earlier observation
                    // standing.
                    return Err(error);
                }
                recorder.and_then(|recorder| {
                    recorder.finish_label(
                        capture.as_ref(),
                        &hex_digest(content.as_bytes()),
                        content.len() as u64,
                    )
                })
            }
            // Guarded replace: the file must exist and its fresh re-read
            // must still hash to the digest the model observed. A missing
            // file is not a create, and a changed file is never clobbered.
            // With a recorder the file is read first, so the digest
            // precondition is checked before any snapshot is prepared and
            // the capture holds the exact before-image.
            Some(digest) => {
                // The precondition is checked against this tool's own fresh
                // read whatever the capture state: a stale digest refuses
                // with the file's current size and digest, and a missing file
                // is not a create. The read is also the capture's
                // before-image when a recorder is attached.
                let file = workspace
                    .read(rel, WORKSPACE_EDIT_MAX_FILE_BYTES)
                    .map_err(|error| {
                        if error.is_not_found() {
                            ToolError::WorkspaceWrite(MISSING_FILE_NOT_A_REPLACE.to_owned())
                        } else {
                            ToolError::WorkspaceWrite(error.to_string())
                        }
                    })?;
                // A stale digest is re-anchored only when this run's own
                // earlier edit/write is the only reason the file moved; a
                // change by anyone else still refuses. A whole-file write
                // has no anchor to re-resolve, but it replaces every byte,
                // so the run's own later state is never partially merged.
                let observed_digest = if file.digest == digest {
                    digest.clone()
                } else if self.read_witnesses.accepts_stale_digest(
                    rel,
                    workspace,
                    &digest,
                    &file.digest,
                ) {
                    file.digest.clone()
                } else {
                    self.read_witnesses.invalidate_if_stale(
                        rel,
                        workspace,
                        &file.digest,
                        file.size,
                    );
                    return Err(write_moved(workspace, rel, &digest, file.size));
                };
                from_digest = Some(observed_digest.clone());
                let capture = recorder.map(|recorder| {
                    recorder.begin(rel, EditSource::WorkspaceWrite, Some(&file.bytes))
                });
                if let Some(text) = capture.as_ref().and_then(Capture::refused_text) {
                    return Err(ToolError::WorkspaceWrite(text.to_owned()));
                }
                if let Err(error) = workspace
                    .write_guarded(rel, &observed_digest, content.as_bytes())
                    .map_err(|error| {
                        if error.is_not_found() {
                            ToolError::WorkspaceWrite(MISSING_FILE_NOT_A_REPLACE.to_owned())
                        } else {
                            match error {
                                HarnessError::ContentChanged { .. } => {
                                    write_moved(workspace, rel, &observed_digest, file.size)
                                }
                                other => ToolError::WorkspaceWrite(other.to_string()),
                            }
                        }
                    })
                {
                    if let (Some(recorder), Some(Capture::Recorded(pending))) =
                        (recorder, capture.as_ref())
                    {
                        recorder.settle(pending, classify_write(&error));
                    }
                    // A failed write leaves the witness alone.
                    return Err(error);
                }
                recorder.and_then(|recorder| {
                    recorder.finish_label(
                        capture.as_ref(),
                        &hex_digest(content.as_bytes()),
                        content.len() as u64,
                    )
                })
            }
        };
        // The digest of what landed is the digest of the bytes the call
        // carried — the harness published exactly these bytes — so no second
        // read is needed; the model chains this digest into a later edit.
        let mut result = serde_json::json!({
            "path": rel,
            "bytes_written": content.len(),
            "size": content.len(),
            "digest": hex_digest(content.as_bytes()),
        });
        if let Some(excerpt) = head_excerpt(content) {
            result["changed_excerpt"] = serde_json::Value::String(excerpt);
        }
        if let Some(label) = label {
            result["recovery"] = serde_json::Value::String(label);
        }
        self.read_witnesses.remember_mutation(
            rel,
            workspace,
            from_digest.as_deref(),
            hex_digest(content.as_bytes()),
            content.len() as u64,
        );
        Ok(result)
    }
}
