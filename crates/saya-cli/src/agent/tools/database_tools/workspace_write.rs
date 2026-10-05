//! The one file-writing tool: an atomic, bounded, create-if-absent write
//! into the run workspace. Path validation, symlink refusal, the prefix
//! check, and the temp-then-rename publication are the harness's (`saya_
//! harness::workspace::Workspace::create_new` and `::write_guarded`) — never
//! re-derived here. This tool's own jobs are argument typing, the byte
//! bound, the `expected_digest` precondition, and surfacing the harness's
//! containment reason as the error text.

use saya_agent::ToolError;
use saya_harness::HarnessError;

use super::DatabaseTools;
use super::redaction_guard;
use super::workspace_edit_anchor::hex_digest;

/// The bound on a workspace write's `content`, matched to
/// `WORKSPACE_READ_MAX_BYTES` (64 KiB) so anything this tool writes can be
/// read back whole in a single `workspace_read` call — the workspace
/// round-trips at most this many bytes through the model's tools in either
/// direction. It also keeps the call's serialized arguments comfortably
/// within every provider's per-message tool-call budget, so a write is never
/// the call the provider silently clips. Over the bound is a typed refusal —
/// never a truncated write: a partially written file is worse than a refused
/// one, and the model can split large content across several files instead.
pub(crate) const WORKSPACE_WRITE_MAX_BYTES: usize = 64 * 1024;

/// The refusal when a no-digest write names an existing entry. It tells the
/// model how to replace, and never carries the file's current digest or
/// content — the model has not been shown the file's state yet.
const FILE_EXISTS_REQUIRES_DIGEST: &str = "file already exists; replacing it requires \
    expected_digest from a fresh workspace_read (or the previous write/edit result); \
    the file was not changed";

/// The refusal when the guarded replace's fresh re-read hashes to a digest
/// the model did not observe: the file moved since it was seen. No digests
/// in the text — the model re-reads with `workspace_read` and retries.
const FILE_CHANGED_SINCE_OBSERVED: &str = "the file changed since it was observed; \
    re-read it with workspace_read and retry; the file was not changed";

/// The refusal when a digest-carrying write names an absent path: a guarded
/// replace is not a create — omitting `expected_digest` is the create form.
const MISSING_FILE_NOT_A_REPLACE: &str =
    "the file does not exist; omit expected_digest to create it";

impl DatabaseTools {
    /// Writes one file into the run workspace under containment:
    /// create-if-absent by default, and a replacement only with the
    /// `expected_digest` the model observed from a fresh read — a stale or
    /// same-size-changed file refuses, so the model is never
    /// last-writer-wins. Like the workspace read tools this never touches a
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
        // An explicit null counts as absent, as in workspace_edit's parser.
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
        match expected_digest {
            // Create-if-absent: an existing entry refuses whole and is left
            // byte-identical; replacing goes through the guarded branch.
            None => {
                workspace
                    .create_new(rel, content.as_bytes())
                    .map_err(|error| match error {
                        HarnessError::AlreadyExists { .. } => {
                            ToolError::WorkspaceWrite(FILE_EXISTS_REQUIRES_DIGEST.to_owned())
                        }
                        other => ToolError::WorkspaceWrite(other.to_string()),
                    })?;
            }
            // Guarded replace: the file must exist and its fresh re-read
            // must still hash to the digest the model observed. A missing
            // file is not a create, and a changed file is never clobbered.
            Some(digest) => {
                workspace
                    .write_guarded(rel, &digest, content.as_bytes())
                    .map_err(|error| {
                        if error.is_not_found() {
                            ToolError::WorkspaceWrite(MISSING_FILE_NOT_A_REPLACE.to_owned())
                        } else {
                            match error {
                                HarnessError::ContentChanged { .. } => ToolError::WorkspaceWrite(
                                    FILE_CHANGED_SINCE_OBSERVED.to_owned(),
                                ),
                                other => ToolError::WorkspaceWrite(other.to_string()),
                            }
                        }
                    })?;
            }
        }
        // The digest of what landed is the digest of the bytes the call
        // carried — the harness published exactly these bytes — so no second
        // read is needed; the model chains this digest into a later edit.
        Ok(serde_json::json!({
            "path": rel,
            "bytes_written": content.len(),
            "size": content.len(),
            "digest": hex_digest(content.as_bytes()),
        }))
    }
}
