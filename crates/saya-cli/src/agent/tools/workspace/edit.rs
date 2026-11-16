//! The model-facing edit has three forms: replace resolves one anchor,
//! append splices at EOF under an offset guard, and multi resolves up to 32
//! non-overlapping anchors against one original snapshot. `replace_all` is
//! the one-anchor rename across many sites: every non-overlapping occurrence
//! of `old_text` is spliced against the one snapshot and the whole result is
//! published once, while the default still refuses an anchor that occurs more
//! than once. Existing-file
//! edits require an explicit digest or the current invocation's completed
//! read witness. Single-range edits use `Workspace::patch_range_guarded`;
//! multi and `replace_all` publish their fully guarded final bytes once. Path
//! resolution, the no-follow
//! open, and the atomic commit are the harness's — never re-derived here.
//! This tool's own jobs are argument typing, the byte bounds, the
//! `expected_*` preconditions, and typed refusals that carry counts, line
//! numbers, sizes and digests — never file content. A stale `expected_digest`
//! that moved only because of this run's own earlier write or edit of the
//! same path re-anchors against the current content, provided the anchor
//! still matches exactly once; a change by anyone else still refuses.
//!
//! Failure contract (DESIGN.md §4): every row refuses loudly and writes nothing.
//! Replacing or appending to an existing file without an explicit digest or
//! the read witness of an earlier read in this run refuses
//! (`WorkspaceDigestRequired`; no write, and the refusal names the file's
//! current size and digest so the model retries without a separate read).
//! Zero matches refuses (`NoMatch`, with the current
//! size and digest so the model re-anchors); multiple matches refuses
//! (`Ambiguous`, with bounded line numbers only, excerpts within 2 KiB —
//! never "first wins"); a moved anchor refuses (`expected_size` or
//! `expected_digest` not matching the current file — no write); a
//! replacement or chunk over the bound refuses whole, never truncated; an
//! offset mismatch refuses (`OffsetMismatch`, reporting the current size and
//! digest so the model resumes from `offset`); a mid-edit truncation lands
//! nothing (a truncated tool-call argument never parses or validates; a
//! crashed commit leaves the old file — temp+rename is atomic with
//! post-write re-verification); a non-UTF-8 target is refused (`NotText` —
//! reads are lossy, so byte-exact anchors cannot be trusted); concurrent writers
//! are refused — never last-writer-wins — because both variants commit behind
//! the guarded fresh-read check, so a same-size change between the tool's own
//! read and the commit is refused as the moved-anchor refusal (no locking, no
//! merge); a creation race on the absent path refuses typed and never
//! clobbers the winner's bytes; an empty anchor is rejected (it matches
//! everywhere).
//!
//! What this does not do: it does not recover from an output-token cap on
//! its own — the continuation loop is unbuilt, so a stopped response resumes
//! only when the model is told to resume from the reported size and digest.
//! Chunking does not beat the context window: every continuation re-sends
//! history, so a task needing more total output than the window allows still
//! fails. `write` remains for small whole-file writes; this is not
//! its replacement.

use saya_agent::{CancellationToken, ToolError};

use super::super::DatabaseTools;

mod anchor;
mod append;
mod args;
mod hint;
mod multi;
mod replace;
mod replace_all;
mod shared;

pub(super) use anchor::hex_digest;
// The commit seam and the size probe, re-exported for the notebook edit,
// which is this same write path over one cell's byte range.
pub(super) use shared::{commit_splice, workspace_size};

pub(super) fn validate_arguments(arguments: &serde_json::Value) -> Result<(), ToolError> {
    args::parse_arguments(arguments).map(|_| ())
}

/// The bound on a workspace edit's `old_text` and `new_text`: the harness
/// patch layer's replacement bound, so anything an edit inserts is accepted
/// by the commit seam. Over the bound is a typed whole-refusal, never a
/// truncation. This is deliberately *not* the whole-file `write` bound of
/// 1 MiB: one write carries a whole file, while a file past the write
/// bound is still built by appending chunks under this bound.
pub(crate) const WORKSPACE_EDIT_MAX_BYTES: usize =
    saya_harness::workspace::patch::PATCH_REPLACEMENT_MAX_BYTES;

/// The bound on a workspace edit's target file: the harness reads the file
/// whole to anchor the match, so targets are capped at the patch layer's
/// file cap with a typed refusal naming the size and the cap. Larger files
/// stay editable through the process lanes under their own gates.
pub(crate) const WORKSPACE_EDIT_MAX_FILE_BYTES: u64 =
    saya_harness::workspace::patch::PATCH_MAX_FILE_BYTES;

pub(in crate::agent::tools) const MULTI_EDIT_OVERLAP_REFUSAL: &str =
    "edit anchors overlap; the file was not changed";

pub(super) enum WorkspaceEditRequest {
    Replace {
        path: String,
        old_text: String,
        new_text: String,
        /// Replace every non-overlapping occurrence of the one anchor instead
        /// of refusing when it occurs more than once. Default `false`, and
        /// admitted only with `old_text`/`new_text`.
        replace_all: bool,
        expected_size: Option<u64>,
        /// The digest the model observed from a fresh `read` or the previous
        /// write/edit result. `None` is admitted here: the refusal has to
        /// name the file's current size and digest, which only the execute
        /// half can read.
        expected_digest: Option<String>,
    },
    Append {
        path: String,
        offset: u64,
        chunk: String,
        expected_size: Option<u64>,
        expected_digest: Option<String>,
    },
    Multi {
        path: String,
        edits: Vec<(String, String)>,
        expected_size: Option<u64>,
        expected_digest: Option<String>,
    },
}

impl DatabaseTools {
    /// Replaces the single occurrence of `old_text` in the contained file
    /// `path` with `new_text`, atomically: zero or multiple matches refuse
    /// with a typed error and write nothing — never "first wins".
    /// Replacing requires `expected_digest`, the digest the model observed
    /// from a fresh read; a file whose current state does not match
    /// what the model measured refuses with a typed error, so a moved
    /// anchor cannot splice against stale offsets. With `offset`+`chunk`
    /// instead, appends `chunk` at EOF under the positional precondition
    /// `offset == current size` (offset 0 on an absent path creates the
    /// file): appending to an existing file requires the same observed
    /// `expected_digest`, and a mismatch refuses with the current size so
    /// the model resumes from `offset` rather than guessing. Both variants
    /// commit behind the guarded fresh-read check —
    /// `Workspace::patch_range_guarded` — so a same-size change between
    /// the tool's own read and the commit is refused as the moved-anchor
    /// refusal; the absent path's offset-0 creation goes through
    /// `Workspace::create_new`.
    /// Like the workspace read tools this never touches a connection —
    /// dispatch routes it before connection resolution — and it records
    /// nothing into the knowledge store: a workspace edit is not evidence
    /// about a database object. With no workspace attached it denies with a
    /// typed error: no workspace, no edit.
    pub(super) async fn workspace_edit(
        &self,
        arguments: &serde_json::Value,
        cancellation: &CancellationToken,
    ) -> Result<serde_json::Value, ToolError> {
        // Each form records its own mutation chain at the point where it
        // knows the content it applied against; the wrapper only dispatches.
        self.workspace_edit_inner(arguments, cancellation).await
    }

    async fn workspace_edit_inner(
        &self,
        arguments: &serde_json::Value,
        cancellation: &CancellationToken,
    ) -> Result<serde_json::Value, ToolError> {
        match args::parse_arguments(arguments)? {
            WorkspaceEditRequest::Replace {
                path,
                old_text,
                new_text,
                replace_all,
                expected_size,
                expected_digest,
            } => {
                self.workspace_replace(
                    &path,
                    &old_text,
                    &new_text,
                    replace_all,
                    expected_size,
                    expected_digest.as_deref(),
                )
                .await
            }
            WorkspaceEditRequest::Append {
                path,
                offset,
                chunk,
                expected_size,
                expected_digest,
            } => {
                self.workspace_append(&path, offset, &chunk, expected_size, expected_digest)
                    .await
            }
            WorkspaceEditRequest::Multi {
                path,
                edits,
                expected_size,
                expected_digest,
            } => {
                self.workspace_multi_edit(
                    &path,
                    &edits,
                    expected_size,
                    expected_digest.as_deref(),
                    cancellation,
                )
                .await
            }
        }
    }
}
