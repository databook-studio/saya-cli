//! The session's digest-guarded file tools, `workspace_write` and
//! `workspace_edit`: derived from the run surface's own definitions so the
//! parameter schemas can never drift, with the run's wording replaced by
//! the session's and the run's plan-gated effect replaced by the per-call
//! ask — the engine decides every call, per `session_definitions`' rule
//! for everything it pushes.

use saya_agent::ToolDefinition;

use super::session_definitions::write_shape;
use crate::agent::tools::DatabaseTools;

/// The run surface's file tools, built with the workspace-write permit on
/// and every other permit off: the entry the session derives from carries
/// the exact parameter schema the executor validates, so advertising it on
/// the session surface can never drift from what dispatch accepts.
fn run_file_definitions() -> Vec<ToolDefinition> {
    DatabaseTools::definitions_with_chart_save(false, false, false, true, false, false)
}

/// `workspace_write`, bound to the session's workspace — the run
/// definition's schema, the session's wording and per-call ask. Replacing
/// an existing file requires the `expected_digest` observed from a fresh
/// read or the previous write/edit result; creating a new file omits it.
pub(crate) fn workspace_write() -> ToolDefinition {
    let mut definition = run_file_definitions()
        .into_iter()
        .find(|definition| definition.name == "workspace_write")
        .expect(
            "workspace_write is a run-surface file tool the session derives from; \
            a missing name is a programming error the schema tests pin",
        );
    definition.description = "Write one whole file into this session's workspace — the project \
        tree the session is bound to; its canonical root is shown in the status header. Pass \
        `path` relative to the workspace root and `content` as the full text to store. Creating \
        a new file omits `expected_digest`; replacing an existing file REQUIRES \
        `expected_digest` — the sha256 `digest` observed from a fresh `workspace_read` or from \
        the previous write/edit result — and the write is refused when the file has changed \
        since, so a concurrent writer's bytes are never clobbered. The write is atomic — \
        replaced whole or not at all, never partially. Absolute paths, `..` escapes, and \
        symlinks are refused, and content over the write bound is refused whole. Returns the \
        `path`, `bytes_written`, `size`, and `digest`. Every call is approved by the user under \
        the session's approval mode."
        .into();
    definition.effect = write_shape(false);
    definition
}

/// `workspace_edit`, beside `workspace_write` on the session surface: the
/// run definition's anchored replace and offset-checked append, with the
/// session's wording and per-call ask. Prefer it for small changes.
pub(crate) fn workspace_edit() -> ToolDefinition {
    let mut definition = run_file_definitions()
        .into_iter()
        .find(|definition| definition.name == "workspace_edit")
        .expect(
            "workspace_edit is a run-surface file tool the session derives from; \
            a missing name is a programming error the schema tests pin",
        );
    definition.description = "Make one small anchored edit in this session's workspace. Pass \
        `path` relative to the workspace root, `old_text` as the exact text to find, \
        `new_text` as its replacement; the anchor must match exactly once: zero or multiple \
        matches refuse, changing nothing. Replacing an existing file requires \
        `expected_digest` — the `digest` from a fresh `workspace_read` or the previous \
        edit's result — and a file that changed since refuses with no write. Or append one \
        `chunk` at an `offset` — the file size you measured; appending to an existing file \
        also requires `expected_digest`, and `offset` must equal the current size: a \
        mismatch refuses with the current size and digest; offset 0 on an absent path \
        creates the file. Anchor, replacement, and chunk over the bound refuse whole. \
        Prefer this over `workspace_write` for small changes; every call is approved by the \
        user under the session's approval mode."
        .into();
    definition.effect = write_shape(false);
    definition
}

#[cfg(test)]
#[path = "session_file_definitions_tests.rs"]
mod session_file_definitions_tests;
