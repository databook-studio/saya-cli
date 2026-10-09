/// The placement guard now runs against the project: a checked-in tool
/// directory is inside the session's fs root and refuses, naming the
/// directory and the root it sits inside. `<project>` itself refuses too.
#[cfg(not(windows))]
#[test]
fn a_program_dir_inside_the_session_workspace_refuses_naming_directory_and_root() {
    let project = worktree("placement");
    let state = temp_dir("placement-state");
    let tools = project.join("tools");
    fs::create_dir_all(&tools).unwrap();
    let runtime = session_runtime(Some((vec!["bench".to_string()], Some(tools.clone()))));
    let error = compose_result(&runtime, &project, &state)
        .map(|_: SessionUniverse| ())
        .expect_err("a checked-in tool directory refuses for sessions");
    assert!(
        error.contains(tools.canonicalize().unwrap().display().to_string().as_str()),
        "the refusal names the directory: {error}"
    );
    assert!(
        error.contains(
            project
                .canonicalize()
                .unwrap()
                .display()
                .to_string()
                .as_str()
        ),
        "the refusal names the root it sits inside: {error}"
    );
    assert!(
        error.contains("cannot express an exclusion"),
        "the refusal says why: {error}"
    );

    // The project root itself as program_dir: the same refusal.
    let error = compose_result(
        &session_runtime(Some((vec!["bench".to_string()], Some(project.clone())))),
        &project,
        &state,
    )
    .map(|_: SessionUniverse| ())
    .expect_err("the workspace root itself as program dir refuses");
    assert!(
        error.contains("overlaps this session's workspace root"),
        "the refusal states the containment: {error}"
    );
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

// ---------------------------------------------------------------------------
// red test 5 — no file tool reaches sessions/<id>/
// ---------------------------------------------------------------------------

/// The session's state directory is outside the workspace root and outside
/// every child's `fs_roots`: a sentinel planted there is unreachable by the
/// file tools — a planted symlink inside the project pointing at it is
/// refused at argument validation (symlink components), relative paths
/// cannot climb out, absolute paths are refused — and the runner
/// composition's only fs root is the workspace root itself.
#[cfg(unix)]
#[tokio::test]
async fn no_file_tool_reaches_the_session_state_dir() {
    use std::os::unix::fs::symlink;
    let project = worktree("sentinel");
    let state = temp_dir("sentinel-state");
    let sentinel = state.join("sentinel.txt");
    fs::write(&sentinel, b"STATE_SENTINEL_9f3a\n").unwrap();
    let universe = compose(&session_runtime(None), &project, &state);

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
    let executor = universe.executor(Arc::clone(&database), &CancellationToken::new(), false);
    let read = executor
        .execute(
            "workspace_read",
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
        let result = executor.execute("workspace_read", escape).await;
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

// ---------------------------------------------------------------------------
// red test 4 — a child's cwd is the workspace root; its writes land in the
// project where the user can see them
// ---------------------------------------------------------------------------

/// On a proven host, the session's runner composes once — one fs root (the
/// workspace), empty egress, the placement guard, the probe — and a child's
/// cwd pins to that root: a staged program writing a relative path lands at
/// the project root, where the user and `git status` see it. The program
/// directory is a real operator-staged binary tree (a copy of a system
/// binary is refused by macOS's signature enforcement under sandbox-exec,
/// so the staging contract's own discipline — real, operator-owned files —
/// is what executes).
#[cfg(target_os = "macos")]
#[tokio::test]
async fn a_session_child_runs_with_the_workspace_root_as_its_cwd() {
    let project = worktree("child-cwd");
    let state = temp_dir("child-state");
    let runtime = session_runtime(Some((
        vec!["touch".to_string()],
        Some(PathBuf::from("/usr/bin")),
    )));
    let universe = compose(&runtime, &project, &state);
    let runner = universe
        .runner
        .as_ref()
        .expect("the probe proves this host (macOS)");
    assert_eq!(
        runner.spawn.fs_roots(),
        &[project.canonicalize().unwrap()],
        "one fs root: the workspace, and nothing else — never the session state dir"
    );

    let database = Arc::new(crate::agent::tools::DatabaseTools::new(None, 100, true));
    let executor = universe.executor(Arc::clone(&database), &CancellationToken::new(), false);
    let result = executor
        .execute(
            "run_program",
            serde_json::json!({"program": "touch", "args": ["out.json"]}),
        )
        .await
        .expect("the staged program runs in the project root");
    let value = serde_json::to_value(&result).unwrap();
    assert!(value.get("error").is_none(), "the child ran: {value}");
    // The relative path landed at the workspace root — the cwd proof — and
    // the file is visible in the project.
    assert!(
        project.join("out.json").exists(),
        "the child's write landed in the project, not a state dir"
    );
    // The outcome record did not create a saya directory in the project.
    assert!(
        !project.join("run_program").exists(),
        "the project gains no saya-created directories"
    );
    let _ = fs::remove_dir_all(&project);
    let _ = fs::remove_dir_all(&state);
}
