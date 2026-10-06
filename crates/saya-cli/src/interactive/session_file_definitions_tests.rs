//! The session file tools' tests: derive parity against the run surface's
//! own definitions, the session's write shape, and the honest wording —
//! the regression this module exists for is a session schema that drifts
//! from the executor, as the old hand-written `workspace_write` did by
//! hiding `expected_digest` while promising bare replacement.

use super::*;
use saya_agent::{LocalStateEffect, ToolConcurrency, ToolDefinition};

use crate::agent::tools::DatabaseTools;

/// The run surface's file-tool definition the session derives from: the
/// same lookup the production helper performs, spelled out here so a
/// missing name fails the test before it fails a session composition.
fn run_file_tool(name: &str) -> ToolDefinition {
    DatabaseTools::definitions_with_chart_save(false, false, false, true, false, false)
        .into_iter()
        .find(|definition| definition.name == name)
        .unwrap_or_else(|| panic!("{name} is a run-surface file tool"))
}

fn assert_derive_parity(session: &ToolDefinition, run: &ToolDefinition) {
    assert_eq!(session.name, run.name);
    assert_eq!(
        session.parameters, run.parameters,
        "{}'s parameter schema is derived from the run definition, never \
         hand-written, so it cannot drift from the executor's",
        session.name
    );
    assert_eq!(session.completion, run.completion);
    assert_eq!(session.read_only, run.read_only);
    assert_eq!(session.concurrency, run.concurrency);
}

/// The session's `workspace_write` advertises the run surface's exact
/// schema — including `expected_digest`, the guard whose absence in the
/// old hand-written session definition told the model it could replace
/// existing files the executor now refuses without an observed digest.
#[test]
fn the_session_s_workspace_write_schema_is_the_run_definition_s_verbatim() {
    let session = workspace_write();
    let run = run_file_tool("workspace_write");
    assert_derive_parity(&session, &run);
    assert!(
        session.parameters["properties"]["expected_digest"].is_object(),
        "the digest guard is advertised: {:?}",
        session.parameters["properties"]
    );
}

/// The same derive parity for `workspace_edit`, advertised on the session
/// surface for the first time: the anchored replace and the offset-checked
/// append ride the run schema verbatim.
#[test]
fn the_session_s_workspace_edit_schema_is_the_run_definition_s_verbatim() {
    let session = workspace_edit();
    let run = run_file_tool("workspace_edit");
    assert_derive_parity(&session, &run);
    assert!(
        session.parameters["properties"]["expected_digest"].is_object(),
        "the digest guard is advertised: {:?}",
        session.parameters["properties"]
    );
    assert!(
        session.parameters["properties"]["old_text"].is_object(),
        "the anchored replace is advertised: {:?}",
        session.parameters["properties"]
    );
}

/// Both file tools carry the session's write shape: the per-call ask the
/// engine resolves (never the run's plan-gated effect), WriteWorkspace, no
/// external side effect, and serial execution.
#[test]
fn both_file_tools_carry_the_session_s_write_shape() {
    for definition in [workspace_write(), workspace_edit()] {
        assert!(
            definition.effect.requires_approval,
            "{} is ask-gated: the engine decides every call",
            definition.name
        );
        assert_eq!(
            definition.effect.local_state,
            LocalStateEffect::WriteWorkspace,
            "{} writes the workspace",
            definition.name
        );
        assert!(
            !definition.effect.external_side_effect,
            "{} stays inside the workspace: no external side effect",
            definition.name
        );
        assert!(!definition.read_only, "{} writes", definition.name);
        assert_eq!(
            definition.concurrency,
            ToolConcurrency::Serial,
            "{} runs serial",
            definition.name
        );
    }
}

/// The session wording is honest where the old definition lied: the write
/// names the digest guard and never promises that existing files are
/// replaced without it.
#[test]
fn the_write_description_names_the_digest_guard_and_never_promises_bare_replacement() {
    let description = workspace_write().description;
    assert!(
        description.contains("expected_digest"),
        "the guard is named: {description}"
    );
    assert!(
        !description.contains("existing files are replaced"),
        "the false promise is gone: {description}"
    );
}

/// The edit's wording names its anchor and its guard: the model is told
/// what matches and what the precondition is.
#[test]
fn the_edit_description_names_the_anchor_and_the_guard() {
    let description = workspace_edit().description;
    assert!(
        description.contains("old_text"),
        "the anchor: {description}"
    );
    assert!(
        description.contains("expected_digest"),
        "the guard: {description}"
    );
}

/// Both descriptions speak for the session — never the run: no "run's
/// workspace" rides along from a borrowed sentence, and each names whose
/// workspace it writes.
#[test]
fn the_descriptions_speak_for_the_session_never_the_run() {
    for definition in [workspace_write(), workspace_edit()] {
        assert!(
            !definition.description.contains("run's workspace"),
            "{} speaks for the session: {}",
            definition.name,
            definition.description
        );
        assert!(
            definition.description.contains("session"),
            "{} names whose workspace it writes: {}",
            definition.name,
            definition.description
        );
    }
}
