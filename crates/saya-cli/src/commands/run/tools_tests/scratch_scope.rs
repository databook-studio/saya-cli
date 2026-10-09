/// The inverse pin's second half: a step whose capabilities ask for scratch
/// carries `scratch_sql` in its definitions — appended after the database
/// universe — while a sibling step in the same plan that did not ask never
/// sees it, in either its definitions or its executor. The run-level
/// admission alone (the run approved scratch) must not leak the tool into
/// the non-asking step.
#[tokio::test]
async fn the_scratch_tool_is_in_the_universe_of_the_steps_that_asked_for_it() {
    let database = Arc::new(DatabaseTools::new(None, 100, true));
    let workspace = workspace("scratch-universe");
    let (root, scratch) = admitted_scratch("universe");

    let mut scratch_caps = Capabilities::default();
    scratch_caps.scratch = true;
    let built = toolsets(
        ToolsetInputs {
            database: &database,
            scratch: Some(&scratch),
            fetch: None,
            runner: None,
            workspace: &workspace,
            allow_query_data: true,
            cancellation: &cancellation(),
        },
        &[
            step("stage results", scratch_caps.clone()),
            step("read only", Capabilities::default()),
        ],
    );

    // The asking step: scratch_sql, appended last, and no workspace_write —
    // scratch alone is not the workspace-write scope.
    assert_eq!(
        names(&built, 0),
        [OPEN_GATE, SCRATCH_TAIL].concat(),
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
    let workspace = workspace("scratch-ddl");
    let (root, scratch) = admitted_scratch("ddl");

    let mut scratch_caps = Capabilities::default();
    scratch_caps.scratch = true;
    let built = toolsets(
        ToolsetInputs {
            database: &database,
            scratch: Some(&scratch),
            fetch: None,
            runner: None,
            workspace: &workspace,
            allow_query_data: true,
            cancellation: &cancellation(),
        },
        &[step("stage results", scratch_caps.clone())],
    );

    assert_eq!(
        names(&built, 0),
        [OPEN_GATE, SCRATCH_TAIL].concat(),
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

/// The composite dispatches the four fixed harness names — and refuses the
/// ones no member backs exactly as the single executor always did: the
/// database tools' typed `UnsupportedTool`, fail closed. `run_program` is
/// not on this list any more: its member is wired (S3), and its narrowing
/// is pinned in the runner tests below.
#[tokio::test]
async fn the_harness_names_fall_through_to_the_same_typed_refusal() {
    let database = Arc::new(DatabaseTools::new(None, 100, true));
    let workspace = workspace("fall-through");
    let built = toolsets(
        ToolsetInputs {
            database: &database,
            scratch: None,
            fetch: None,
            runner: None,
            workspace: &workspace,
            allow_query_data: true,
            cancellation: &cancellation(),
        },
        &[step("read only", Capabilities::default())],
    );
    for name in ["scratch_sql", "http_fetch", "http_download", "wat"] {
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
