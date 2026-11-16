/// `render_chart` is answered by nothing: no session member claims the name,
/// so the composite's own unknown-name refusal is the answer. A saved session that names it still loads — the name is inert
/// wire data — and a call under it is refused rather than dispatched.
#[tokio::test]
async fn the_session_executor_refuses_the_retired_render_chart() {
    let project = worktree("chart-removed-exec");
    let state = temp_dir("chart-removed-exec-state");
    let tools = temp_dir("chart-removed-exec-tools");
    let runtime = session_runtime();
    let universe = compose(&runtime, &project, &state);
    let database = Arc::new(crate::agent::tools::DatabaseTools::new(None, 100, true));
    let executor = universe.executor(
            Arc::clone(&database),
            &CancellationToken::new(),
            None,
            None,
            None,
            Default::default(),
        );
    let refused = executor
        .execute(
            "render_chart",
            serde_json::json!({"sql": "SELECT 1", "chart_type": "bar"}),
        )
        .await
        .expect_err("render_chart is not a session tool");
    let refusal = format!("{refused:?}");
    assert!(
        refusal.contains("UnsupportedTool"),
        "no member answers the retired name: {refusal}"
    );
    let _ = (
        fs::remove_dir_all(&project),
        fs::remove_dir_all(&state),
        fs::remove_dir_all(&tools),
    );
}

/// The same refusal on the surface `saya ask` composes: `DatabaseTools`
/// alone, with no session member to answer the name.
#[tokio::test]
async fn the_ask_surface_refuses_the_retired_render_chart() {
    let tools = crate::agent::tools::DatabaseTools::new(None, 100, true);
    let refused = tools
        .execute(
            "render_chart",
            serde_json::json!({"sql": "SELECT 1", "chart_type": "bar"}),
        )
        .await
        .expect_err("render_chart is not an ask-surface tool");
    let refusal = format!("{refused:?}");
    assert!(
        refusal.contains("UnsupportedTool"),
        "the retired name is unknown, not merely unpermitted: {refusal}"
    );
}
