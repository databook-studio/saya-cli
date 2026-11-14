// ---------------------------------------------------------------------------
// red test 5 — no file tool reaches sessions/<id>/
// ---------------------------------------------------------------------------

/// The session's state directory is outside the workspace root and outside
/// every child's `fs_roots`: a sentinel planted there is unreachable by the
/// file tools — a planted symlink inside the project pointing at it is
/// refused at argument validation (symlink components), relative paths
/// cannot climb out, absolute paths are refused.
#[cfg(unix)]
#[tokio::test]
async fn no_file_tool_reaches_the_session_state_dir() {
    use std::os::unix::fs::symlink;
    let project = worktree("sentinel");
    let state = temp_dir("sentinel-state");
    let sentinel = state.join("sentinel.txt");
    fs::write(&sentinel, b"STATE_SENTINEL_9f3a\n").unwrap();
    let universe = compose(&session_runtime(), &project, &state);

    // The composition itself: the session state dir sits outside the root.
    let root = universe.root().expect("the worktree binds").to_path_buf();
    assert!(
        !state.starts_with(&root),
        "the session state dir must sit outside the workspace root"
    );

    // The file tools ride the workspace seam. A planted symlink into the
    // state dir inside the *project* is refused at every hop, and the
    // traversal shapes are refused before any filesystem contact.
    let link = project.join("state-link");
    symlink(&state, &link).unwrap();
    let workspace = universe.workspace();
    let database = Arc::new(
        crate::agent::tools::DatabaseTools::new(None, 100, true).with_workspace(workspace),
    );
    let executor = universe.executor(
        Arc::clone(&database),
        &CancellationToken::new(),
        None,
        None,
        None,
        Default::default(),
    );
    let read = executor
        .execute(
            "read",
            serde_json::json!({"path": "state-link/sentinel.txt"}),
        )
        .await
        .expect_err("a symlink into the session state dir is refused");
    let read = format!("{read:?}");
    assert!(
        read.contains("symlink"),
        "the refusal names the symlink: {read}"
    );
    for escape in [
        serde_json::json!({"path": format!("{}/sentinel.txt", state.display())}),
        serde_json::json!({"path": "../sentinel.txt"}),
        serde_json::json!({"path": "../sentinel-state/sentinel.txt"}),
    ] {
        let result = executor.execute("read", escape).await;
        assert!(
            result.is_err(),
            "no relative or absolute path reaches the state dir: {result:?}"
        );
    }
    // The sentinel was never read: it still holds only the planted bytes.
    assert_eq!(
        fs::read(&sentinel).unwrap(),
        b"STATE_SENTINEL_9f3a\n",
        "the sentinel was never rewritten"
    );
    let _ = fs::remove_dir_all(&project);
    let _ = fs::remove_dir_all(&state);
}
