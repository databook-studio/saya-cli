/// The shared-scratch trap: two concurrent sessions over the same project
/// hold separate scratch files, each at its own `sessions/<id>/`, and data
/// staged in one is invisible to the other. The per-session id is the
/// containment boundary for state.
#[tokio::test]
async fn scratch_is_per_session_not_per_project() {
    let project = worktree("scratch-boundary");
    let state_a = temp_dir("scratch-a");
    let state_b = temp_dir("scratch-b");
    let universe_a = compose(&session_runtime(None), &project, &state_a);
    let universe_b = compose(&session_runtime(None), &project, &state_b);
    let scratch_a = universe_a.scratch.as_ref().expect("scratch composed");
    let scratch_b = universe_b.scratch.as_ref().expect("scratch composed");
    scratch_a
        .run("CREATE TABLE stage AS SELECT 42 AS v")
        .await
        .expect("session A stages a table");
    assert!(
        state_a.join("scratch.duckdb").exists() && state_b.join("scratch.duckdb").exists(),
        "each session's scratch lives at its own state dir"
    );
    let leaked = scratch_b.run("SELECT v FROM stage").await;
    assert!(
        leaked.is_err(),
        "session B must not see session A's staged table: {leaked:?}"
    );
    let _ = fs::remove_dir_all(&project);
    let _ = fs::remove_dir_all(&state_a);
    let _ = fs::remove_dir_all(&state_b);
}

/// Recomposition keeps the first universe alive for active TUI snapshots,
/// while both universes share the one DuckDB connection for this session.
#[tokio::test]
async fn recomposing_a_session_reuses_its_live_scratch_connection() {
    let project = worktree("scratch-recompose");
    let state = temp_dir("scratch-recompose-state");
    let runtime = session_runtime(None);
    let first = compose(&runtime, &project, &state);
    let scratch = first.scratch.clone().expect("scratch composed");
    first
        .scratch
        .as_ref()
        .expect("scratch composed")
        .run("CREATE TABLE stage AS SELECT 42 AS v")
        .await
        .expect("the first universe stages a table");

    let second = SessionUniverse::compose_with_launch_and_path(
        crate::interactive::session::universe::SessionComposition {
            runtime: &runtime,
            explicit: None,
            pinned_root: None,
            walk_when_unpinned: true,
            cwd: &project,
            state_dir: &state,
            launch: None,
            path: Some("test-path".to_owned()),
            scratch: Some(scratch),
            recovery: None,
        },
    )
    .expect("recomposition shares the live scratch database");

    let second_scratch = second.scratch.as_ref().expect("scratch composed");
    let rows = second_scratch
        .run("SELECT v FROM stage")
        .await
        .expect("the second universe reads the first universe's stage");
    assert_eq!(rows.rows.len(), 1, "the staged row remains available");
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// A later unbound universe retains scratch state but must clear the prior
/// workspace binding, hiding the CSV importer from its tool surface.
#[tokio::test]
async fn recomposing_unbound_clears_scratch_import_access() {
    let project = worktree("scratch-recompose-unbound");
    let plain = temp_dir("scratch-recompose-plain");
    let state = temp_dir("scratch-recompose-unbound-state");
    let runtime = session_runtime(None);
    let first = compose(&runtime, &project, &state);
    first
        .scratch
        .as_ref()
        .expect("scratch composed")
        .run("CREATE TABLE stage AS SELECT 42 AS v")
        .await
        .expect("the bound universe stages a table");

    let second = SessionUniverse::compose_with_launch_and_path(
        crate::interactive::session::universe::SessionComposition {
            runtime: &runtime,
            explicit: None,
            pinned_root: None,
            walk_when_unpinned: false,
            cwd: &plain,
            state_dir: &state,
            launch: None,
            path: Some("test-path".to_owned()),
            scratch: first.scratch(),
            recovery: None,
        },
    )
    .expect("the unbound recomposition retains scratch");

    let names = advertised(&second, AgentMode::Build, ApprovalPolicy::Ask, true);
    assert!(
        !names.contains(&"scratch_import".to_owned()),
        "an unbound universe hides scratch imports: {names:?}"
    );
    second
        .scratch
        .as_ref()
        .expect("scratch composed")
        .execute(
            "scratch_import",
            serde_json::json!({"path": "data.csv", "table": "stage"}),
        )
        .await
        .expect_err("an unbound universe refuses scratch imports");
    let rows = second
        .scratch
        .as_ref()
        .expect("scratch composed")
        .run("SELECT v FROM stage")
        .await
        .expect("the shared scratch database retains staged data");
    assert_eq!(rows.rows.len(), 1, "the staged row remains available");
    let _ = (
        fs::remove_dir_all(&project),
        fs::remove_dir_all(&plain),
        fs::remove_dir_all(&state),
    );
}
