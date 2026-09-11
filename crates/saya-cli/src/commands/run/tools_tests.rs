//! The toolset seam's gate: the per-step toolsets must carry the run's tool
//! universe byte-identically to the single executor and run-level universe
//! they replace. If a definition appears, disappears, or reorders — in any
//! step, under any approved-scope shape — the seam is wrong.

use std::sync::Arc;

use saya_agent::{LocalStateEffect, ToolError};
use saya_types::Capabilities;

use crate::agent::tools::DatabaseTools;

use super::tools::toolsets;

/// The names each step's definitions must carry, in order, for a read-only
/// run with the privacy gate open: the database and workspace-read set, no
/// write tool, and no contract tools (a run passes no state store).
/// `workspace_write` sits between `grep` and the sql tools when approved.
const OPEN_GATE: &[&str] = &[
    "schema_discovery",
    "workspace_read",
    "workspace_list",
    "glob",
    "grep",
    "bounded_sql_query",
    "bounded_sql_query_all",
    "result_shape",
    "column_health",
    "join_check",
    "render_chart",
    "designate_answer",
];

/// The same universe with the privacy gate closed: every tool that touches
/// database data is hidden.
const CLOSED_GATE: &[&str] = &[
    "schema_discovery",
    "workspace_read",
    "workspace_list",
    "glob",
    "grep",
];

fn names(toolsets: &[saya_harness::engine::StepToolset], step: usize) -> Vec<String> {
    toolsets[step]
        .definitions
        .iter()
        .map(|definition| definition.name.clone())
        .collect()
}

/// The expected names are stated here, not derived from the builder's own
/// construction, so a drift is a diff in this test rather than a silent
/// universe change. Every step's toolset is asserted equal — same names, in
/// order, and the same serialized definitions — because this seam gives
/// every step exactly the universe the single run-level one built.
#[test]
fn every_step_s_definitions_are_the_unchanged_run_universe() {
    let database = Arc::new(DatabaseTools::new(None, 100, true));
    let scopes = Capabilities::default();

    let built = toolsets(&database, true, &scopes, 2);
    for step in 0..2 {
        assert_eq!(
            names(&built, step),
            OPEN_GATE,
            "step {step}'s universe appeared, lost, or reordered a definition"
        );
        // The workspace-write filter still keys on the approved scope, and
        // with none approved every tool is read-shaped or gate-shaped
        // exactly as before: no definition may carry the write effect.
        for definition in &built[step].definitions {
            assert_ne!(
                definition.effect.local_state,
                LocalStateEffect::WriteWorkspace,
                "{} must be hidden from a run without workspace-write",
                definition.name
            );
        }
    }
    // Byte-identical across steps — the serialized definitions agree, not
    // merely the names.
    assert_eq!(
        serde_json::to_string(&built[0].definitions).unwrap(),
        serde_json::to_string(&built[1].definitions).unwrap(),
        "every step's toolset must be the same universe"
    );

    let built = toolsets(&database, false, &scopes, 1);
    assert_eq!(names(&built, 0), CLOSED_GATE);

    let mut write_scopes = Capabilities::default();
    write_scopes.workspace_write = true;
    let built = toolsets(&database, true, &write_scopes, 1);
    assert_eq!(
        names(&built, 0),
        [
            "schema_discovery",
            "workspace_read",
            "workspace_list",
            "glob",
            "grep",
            "workspace_write",
            "bounded_sql_query",
            "bounded_sql_query_all",
            "result_shape",
            "column_health",
            "join_check",
            "render_chart",
            "designate_answer",
        ],
        "the approved write tool must keep its place in the universe"
    );
}

/// The composite dispatches the four fixed harness names — and, until a
/// member executor is wired, refuses them exactly as the single executor
/// always did: the database tools' typed `UnsupportedTool`, fail closed.
#[tokio::test]
async fn the_harness_names_fall_through_to_the_same_typed_refusal() {
    let database = Arc::new(DatabaseTools::new(None, 100, true));
    let built = toolsets(&database, true, &Capabilities::default(), 1);
    for name in [
        "scratch_sql",
        "http_fetch",
        "http_download",
        "run_program",
        "wat",
    ] {
        let error = built[0]
            .executor
            .execute(name, serde_json::json!({}))
            .await
            .expect_err("nothing in this slice can run any of these names");
        assert_eq!(
            error,
            ToolError::UnsupportedTool,
            "`{name}` must be refused with today's typed error, got: {error:?}"
        );
    }
}
