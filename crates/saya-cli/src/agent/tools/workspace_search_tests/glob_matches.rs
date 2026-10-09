#[tokio::test]
async fn glob_matches_contained_paths_with_glob_semantics() {
    let sandbox = Sandbox::new("glob-semantics");
    for path in ["a.md", "notes/b.md", "notes/deep/c.md", "code/x.rs"] {
        sandbox
            .ws
            .write(path, b"content")
            .expect("contained write must succeed");
    }
    let tools = sandbox.tools();
    assert_eq!(
        glob_paths(&tools, "**/*.md").await,
        vec!["a.md", "notes/b.md", "notes/deep/c.md"],
        "`**` spans segments and also matches zero of them"
    );
    assert_eq!(
        glob_paths(&tools, "*.md").await,
        vec!["a.md"],
        "`*` stays inside one segment"
    );
    assert_eq!(glob_paths(&tools, "notes/*.md").await, vec!["notes/b.md"]);
    assert_eq!(
        glob_paths(&tools, "notes/**").await,
        vec!["notes", "notes/b.md", "notes/deep", "notes/deep/c.md"],
        "a trailing `**` matches the directory and everything real under it"
    );
    assert_eq!(
        glob_paths(&tools, "/etc/**").await,
        Vec::<String>::new(),
        "an absolute pattern can never match: only walk-contained paths are candidates"
    );
    assert_eq!(
        glob_paths(&tools, "../outside/**").await,
        Vec::<String>::new()
    );
}

#[tokio::test]
async fn glob_denies_when_no_workspace_is_attached() {
    let tools = DatabaseTools::with_registry(
        crate::connection::ConnectionRegistry::new("primary"),
        100,
        true,
        None,
    );
    let error = tools
        .execute("glob", serde_json::json!({"pattern": "**/*.md"}))
        .await
        .expect_err("no workspace, no glob");
    assert_eq!(error, ToolError::WorkspaceUnavailable);
}

#[tokio::test]
async fn glob_reports_a_match_count_overrun_as_a_typed_error() {
    let sandbox = Sandbox::new("glob-match-bound");
    sandbox.plant(
        (0..WORKSPACE_GLOB_MAX_MATCHES + 1).map(|index| format!("f{index}.log")),
        b"",
    );
    let tools = sandbox.tools();
    let error = tools
        .execute("glob", serde_json::json!({"pattern": "*.log"}))
        .await
        .expect_err("an over-bound match list must be refused, not truncated");
    match error {
        ToolError::Workspace(message) => {
            assert!(
                message.contains("bound exceeded"),
                "the refusal must name the bounds failure: {message}"
            );
        }
        other => panic!("expected a typed workspace error, got: {other}"),
    }
}
