//! The model-facing anchored edit (`replace` and `append` variants): the
//! replace variant resolves `old_text` to exactly one byte range of a
//! contained workspace file; the append variant splices `chunk` at an empty
//! range at EOF guarded by `offset`. Both commit through
//! `Workspace::patch_range` — append *is* a range replace, not a second write
//! path. Path resolution, the no-follow open, and the atomic commit are the
//! harness's — never re-derived here. This tool's own jobs are argument
//! typing, the byte bounds, the `expected_*` precondition, and typed refusals
//! that carry counts, line numbers, sizes and digests — never file content.
//!
//! Failure contract (DESIGN.md §4): every row refuses loudly and writes nothing.
//! Zero matches refuses (`NoMatch`, with the current
//! size and digest so the model re-anchors); multiple matches refuses
//! (`Ambiguous`, with bounded line numbers only, excerpts within 2 KiB —
//! never "first wins"); a moved anchor refuses (`expected_size` or
//! `expected_digest` no longer matching the current file — no write); a
//! replacement or chunk over the bound refuses whole, never truncated; an
//! offset mismatch refuses (`OffsetMismatch`, reporting the current size and
//! digest so the model resumes from `offset`); a mid-edit truncation lands
//! nothing (a truncated tool-call argument never parses or validates; a
//! crashed commit leaves the old file — temp+rename is atomic with
//! post-write re-verification); a non-UTF-8 target is refused (`NotText` —
//! reads are lossy, so byte-exact anchors cannot be trusted); concurrent writers
//! are last-writer-wins unless `expected_*` is supplied, in which
//! case the loser gets the moved-anchor refusal (no locking, no merge); an
//! empty anchor is rejected (it matches everywhere).
//!
//! What this does not do: it does not recover from an output-token cap on
//! its own — the continuation loop is unbuilt, so a stopped response resumes
//! only when the model is told to resume from the reported size and digest.
//! Chunking does not beat the context window: every continuation re-sends
//! history, so a task needing more total output than the window allows still
//! fails. `workspace_write` remains for small whole-file writes; this is not
//! its replacement.

use saya_agent::ToolError;

use super::DatabaseTools;
pub(super) use super::workspace_edit_args::parse_arguments;
use super::workspace_write::WORKSPACE_WRITE_MAX_BYTES;

/// The bound on a workspace edit's `old_text` and `new_text`: the same 64
/// KiB round-trip discipline as the whole-file write, so anything an edit
/// inserts can be read back whole. Over the bound is a typed whole-refusal,
/// never a truncation.
pub(crate) const WORKSPACE_EDIT_MAX_BYTES: usize = WORKSPACE_WRITE_MAX_BYTES;

/// The bound on a workspace edit's target file: the harness reads the file
/// whole to anchor the match, so targets are capped at the patch layer's
/// file cap with a typed refusal naming the size and the cap. Larger files
/// stay editable through the process lanes under their own gates.
pub(crate) const WORKSPACE_EDIT_MAX_FILE_BYTES: u64 =
    saya_harness::workspace::patch::PATCH_MAX_FILE_BYTES;

pub(super) enum WorkspaceEditRequest {
    Replace {
        path: String,
        old_text: String,
        new_text: String,
        expected_size: Option<u64>,
        expected_digest: Option<String>,
    },
    Append {
        path: String,
        offset: u64,
        chunk: String,
        expected_size: Option<u64>,
        expected_digest: Option<String>,
    },
}

impl DatabaseTools {
    /// Replaces the single occurrence of `old_text` in the contained file
    /// `path` with `new_text`, atomically: zero or multiple matches refuse
    /// with a typed error and write nothing — never "first wins". The
    /// optional `expected_size`/`expected_digest` precondition refuses with a
    /// typed error when the file's current state no longer matches what the
    /// model measured, so a moved anchor cannot splice against stale offsets.
    /// With `offset`+`chunk` instead, appends `chunk` at EOF under the
    /// positional precondition `offset == current size` (offset 0 on an
    /// absent path creates the file); a mismatch refuses with the current
    /// size so the model resumes from `offset` rather than guessing. Both
    /// variants commit the same way, through `Workspace::patch_range`:
    /// append is the empty range `size..size` with `offset` as the
    /// precondition, never a second write path.
    /// Like the workspace read tools this never touches a connection —
    /// dispatch routes it before connection resolution — and it records
    /// nothing into the knowledge store: a workspace edit is not evidence
    /// about a database object. With no workspace attached it denies with a
    /// typed error: no workspace, no edit.
    pub(super) async fn workspace_edit(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        match parse_arguments(arguments)? {
            WorkspaceEditRequest::Replace {
                path,
                old_text,
                new_text,
                expected_size,
                expected_digest,
            } => {
                self.workspace_replace(&path, &old_text, &new_text, expected_size, expected_digest)
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
        }
    }
}
