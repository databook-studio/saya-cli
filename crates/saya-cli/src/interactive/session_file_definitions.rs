//! The session's digest-guarded file tools, `write` and
//! `edit`: derived from the run surface's own definitions so the
//! parameter schemas can never drift, with the lean wording's "this run"
//! spoken as "this session" and the run's plan-gated effect replaced by the
//! per-call ask — the engine decides every call, per `session_definitions`'
//! rule for everything it pushes.

use saya_agent::ToolDefinition;

use super::session_definitions::write_shape;
use crate::agent::tools::DatabaseTools;
use crate::agent::tools::lean;

/// The run surface's file tools, built with the workspace-write permit on
/// and every other permit off: the entry the session derives from carries
/// the exact parameter schema the executor validates, so advertising it on
/// the session surface can never drift from what dispatch accepts.
fn run_file_definitions() -> Vec<ToolDefinition> {
    DatabaseTools::definitions(false, false, false, true)
}

/// `write`, bound to the session's workspace — the run
/// definition's schema, the session's wording and per-call ask. Replacing
/// an existing file requires the `expected_digest` observed from a fresh
/// read or the previous write/edit result; creating a new file omits it.
pub(crate) fn workspace_write() -> ToolDefinition {
    let mut definition = run_file_definitions()
        .into_iter()
        .find(|definition| definition.name == "write")
        .expect(
            "write is a run-surface file tool the session derives from; \
            a missing name is a programming error the schema tests pin",
        );
    definition.description = lean::WRITE.replace("this run", "this session");
    definition.effect = write_shape(false);
    definition
}

/// `edit`, beside `write` on the session surface: the
/// run definition's anchored replace and offset-checked append, with the
/// session's wording and per-call ask. Prefer it for small changes.
pub(crate) fn workspace_edit() -> ToolDefinition {
    let mut definition = run_file_definitions()
        .into_iter()
        .find(|definition| definition.name == "edit")
        .expect(
            "edit is a run-surface file tool the session derives from; \
            a missing name is a programming error the schema tests pin",
        );
    definition.description = lean::EDIT.replace("this run", "this session");
    definition.effect = write_shape(false);
    definition
}

#[cfg(test)]
#[path = "session_file_definitions_tests.rs"]
mod session_file_definitions_tests;
