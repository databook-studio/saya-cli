//! The toolset builder's gates: the per-step toolsets must carry the run's
//! database and workspace universe byte-identically to the single executor
//! and run-level universe they replaced — and, since S1, the scratch tool
//! must appear in — and only in — the steps that asked for it, behind a
//! composite that actually runs it.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use saya_agent::{LocalStateEffect, ToolError};
use saya_harness::scratch::ScratchSql;
use saya_types::{Capabilities, StepSpec};

use crate::agent::tools::DatabaseTools;

use super::tools::toolsets;

/// The names each step's definitions must carry, in order, for a read-only
/// run with the privacy gate open: the database and workspace-read set, no
/// write tool, and no contract tools (a run passes no state store).
/// `workspace_write` sits between `grep` and the sql tools when approved;
/// `scratch_sql` is appended last when the step asked for scratch.
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

fn step(goal: &str, capabilities: Capabilities) -> StepSpec {
    StepSpec::new(goal, capabilities, None, Vec::new(), None).unwrap()
}

/// An admitted `ScratchSql` over a private run root, as `assemble` builds
/// it for a run that approved scratch.
fn admitted_scratch(label: &str) -> (PathBuf, Arc<ScratchSql>) {
    let mut capabilities = Capabilities::default();
    capabilities.scratch = true;
    let root = std::env::temp_dir().join(format!("saya-run-tools-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let scratch = ScratchSql::admit(&root, &capabilities)
        .unwrap()
        .expect("the run approved scratch, so admission admits");
    (root, Arc::new(scratch))
}

/// The expected names are stated here, not derived from the builder's own
/// construction, so a drift is a diff in this test rather than a silent
/// universe change. Read-only and write-approving steps keep the exact
/// universe the single run-level one built; a scratch-asking step gets
/// `scratch_sql` appended — and a step that did not ask, in the same plan,
/// never sees it.
#[test]
fn every_step_s_definitions_follow_the_step_s_capabilities() {
    let database = Arc::new(DatabaseTools::new(None, 100, true));

    // Read-only steps: the unchanged universe, byte-identical across steps.
    let built = toolsets(
        &database,
        None,
        true,
        &[
            step("read the schema", Capabilities::default()),
            step("summarize", Capabilities::default()),
        ],
    );
    for step_index in 0..2 {
        assert_eq!(
            names(&built, step_index),
            OPEN_GATE,
            "step {step_index}'s universe appeared, lost, or reordered a definition"
        );
        // The write filter still keys on the approved scopes, and with none
        // approved every tool is read-shaped or gate-shaped exactly as
        // before: no definition may carry the write effect.
        for definition in &built[step_index].definitions {
            assert_ne!(
                definition.effect.local_state,
                LocalStateEffect::WriteWorkspace,
                "{} must be hidden from a run without a write-shaped scope",
                definition.name
            );
        }
    }
    // Byte-identical across steps — the serialized definitions agree, not
    // merely the names.
    assert_eq!(
        serde_json::to_string(&built[0].definitions).unwrap(),
        serde_json::to_string(&built[1].definitions).unwrap(),
        "steps with the same capabilities must carry the same universe"
    );

    let built = toolsets(
        &database,
        None,
        false,
        &[step("closed gate", Capabilities::default())],
    );
    assert_eq!(names(&built, 0), CLOSED_GATE);

    let mut write_scopes = Capabilities::default();
    write_scopes.workspace_write = true;
    let built = toolsets(&database, None, true, &[step("write files", write_scopes)]);
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

/// The inverse pin's second half: a step whose capabilities ask for scratch
/// carries `scratch_sql` in its definitions — appended after the database
/// universe — while a sibling step in the same plan that did not ask never
/// sees it, in either its definitions or its executor. The run-level
/// admission alone (the run approved scratch) must not leak the tool into
/// the non-asking step.
#[tokio::test]
async fn the_scratch_tool_is_in_the_universe_of_the_steps_that_asked_for_it() {
    let database = Arc::new(DatabaseTools::new(None, 100, true));
    let (root, scratch) = admitted_scratch("universe");

    let mut scratch_caps = Capabilities::default();
    scratch_caps.scratch = true;
    let built = toolsets(
        &database,
        Some(&scratch),
        true,
        &[
            step("stage results", scratch_caps.clone()),
            step("read only", Capabilities::default()),
        ],
    );

    // The asking step: scratch_sql, appended last, and no workspace_write —
    // scratch alone is not the workspace-write scope.
    assert_eq!(
        names(&built, 0),
        [OPEN_GATE, &["scratch_sql"]].concat(),
        "the scratch-asking step's universe must end with scratch_sql"
    );
    assert!(
        !names(&built, 0).contains(&"workspace_write".to_string()),
        "scratch alone must not carry the workspace-write tool"
    );
    // The non-asking step: byte-identical to the unchanged universe.
    assert_eq!(
        names(&built, 1),
        OPEN_GATE,
        "a step that did not ask for scratch must never see it"
    );

    // And the executor narrows with the definitions: the non-asking step's
    // composite refuses the name even though the run admitted the scope.
    let error = built[1]
        .executor
        .execute("scratch_sql", serde_json::json!({}))
        .await
        .expect_err("the non-asking step has no scratch member behind its composite");
    assert_eq!(error, ToolError::UnsupportedTool);

    let _ = fs::remove_dir_all(root);
}

/// The slice's headline gate: `--allow scratch` alone — without
/// `workspace-write` — runs DDL through `scratch_sql`. The permit union
/// from S0 carries the write permit for a scratch-only step (pinned in the
/// harness's brief tests); here the toolset builder puts the tool in the
/// step's universe and the composite executes the statement for real.
#[tokio::test]
async fn ddl_runs_through_scratch_sql_with_only_the_scratch_scope() {
    let database = Arc::new(DatabaseTools::new(None, 100, true));
    let (root, scratch) = admitted_scratch("ddl");

    let mut scratch_caps = Capabilities::default();
    scratch_caps.scratch = true;
    let built = toolsets(
        &database,
        Some(&scratch),
        true,
        &[step("stage results", scratch_caps.clone())],
    );

    assert_eq!(
        names(&built, 0),
        [OPEN_GATE, &["scratch_sql"]].concat(),
        "the step's universe must carry scratch_sql and nothing write-shaped besides"
    );

    let executor = &built[0].executor;
    executor
        .execute(
            "scratch_sql",
            serde_json::json!({"sql": "CREATE TABLE staged (a INTEGER)"}),
        )
        .await
        .expect("DDL must run through scratch_sql with scratch alone");
    executor
        .execute(
            "scratch_sql",
            serde_json::json!({"sql": "INSERT INTO staged VALUES (42)"}),
        )
        .await
        .expect("DML must run through scratch_sql with scratch alone");
    let result = executor
        .execute(
            "scratch_sql",
            serde_json::json!({"sql": "SELECT a FROM staged"}),
        )
        .await
        .expect("the staged row must be readable back");
    assert!(
        serde_json::to_string(&result).unwrap().contains("42"),
        "the DDL and DML must have landed in the scratch file, got: {result}"
    );

    let _ = fs::remove_dir_all(root);
}

/// The composite dispatches the four fixed harness names — and refuses them
/// exactly as the single executor always did where no member executor backs
/// them: the database tools' typed `UnsupportedTool`, fail closed.
#[tokio::test]
async fn the_harness_names_fall_through_to_the_same_typed_refusal() {
    let database = Arc::new(DatabaseTools::new(None, 100, true));
    let built = toolsets(
        &database,
        None,
        true,
        &[step("read only", Capabilities::default())],
    );
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
