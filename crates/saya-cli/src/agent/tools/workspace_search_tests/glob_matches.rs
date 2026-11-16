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
    // Which paths match, not the mtime order they arrive in: the fixture's
    // own writes are milliseconds apart, and ordering has its own test.
    assert_eq!(
        sorted(glob_paths(&tools, "**/*.md").await),
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
        sorted(glob_paths(&tools, "notes/**").await),
        vec!["notes", "notes/b.md", "notes/deep", "notes/deep/c.md"],
        "a trailing `**` matches the directory and everything real under it"
    );
    for pattern in ["/etc/**", "../outside/**"] {
        assert!(
            tools
                .execute("glob", serde_json::json!({"pattern":pattern}))
                .await
                .is_err(),
            "unsafe glob paths are refused, never served"
        );
    }
}

/// "What did I just change?" is answerable from one call: the newest path is
/// first, so the model reads the top of `matches` instead of guessing.
#[tokio::test]
async fn glob_serves_the_newest_modified_path_first() {
    let sandbox = Sandbox::new("glob-mtime-order");
    for (path, seconds) in [
        ("src/old.rs", 1_600_000_000),
        ("src/new.rs", 1_600_000_200),
        ("src/mid.rs", 1_600_000_100),
    ] {
        sandbox.ws.write(path, b"content").unwrap();
        set_mtime(&sandbox.ws, path, seconds);
    }
    assert_eq!(
        glob_paths(&sandbox.tools(), "**/*.rs").await,
        vec!["src/new.rs", "src/mid.rs", "src/old.rs"]
    );
}

#[tokio::test]
async fn glob_type_filter_serves_only_the_requested_kind() {
    let sandbox = Sandbox::new("glob-type");
    for path in ["src/main.rs", "src/nested/deep.rs", "notes.txt"] {
        sandbox.ws.write(path, b"content").unwrap();
    }
    let tools = sandbox.tools();
    let call = |arguments: serde_json::Value| {
        let tools = &tools;
        async move {
            tools
                .execute("glob", arguments)
                .await
                .expect("a contained typed glob must succeed")
        }
    };
    let files = call(serde_json::json!({"pattern":"**/*.rs","type":"file"})).await;
    assert_eq!(
        sorted(
            files["matches"]
                .as_array()
                .unwrap()
                .iter()
                .map(|value| value.as_str().unwrap().to_string())
                .collect()
        ),
        ["src/main.rs", "src/nested/deep.rs"]
    );
    let dirs = call(serde_json::json!({"pattern":"**","type":"dir"})).await;
    assert_eq!(
        sorted(
            dirs["matches"]
                .as_array()
                .unwrap()
                .iter()
                .map(|value| value.as_str().unwrap().to_string())
                .collect()
        ),
        ["src", "src/nested"]
    );
    // An unknown kind is refused rather than silently serving everything.
    for refused in [serde_json::json!({"pattern":"**","type":"symlink"}), serde_json::json!({"pattern":"**","type":7})] {
        assert_eq!(
            tools.execute("glob", refused).await.unwrap_err(),
            ToolError::UnsupportedProperty
        );
    }
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
async fn glob_reports_bounded_matches_and_the_exact_total() {
    let sandbox = Sandbox::new("glob-match-bound");
    sandbox.plant(
        (0..WORKSPACE_GLOB_MAX_MATCHES + 1).map(|index| format!("f{index}.log")),
        b"",
    );
    let result = sandbox
        .tools()
        .execute("glob", serde_json::json!({"pattern":"*.log"}))
        .await
        .unwrap();
    assert_eq!(
        result["matches"].as_array().unwrap().len(),
        WORKSPACE_GLOB_MAX_MATCHES
    );
    assert_eq!(result["truncated"], true);
    assert_eq!(result["coverage_complete"], true);
    assert_eq!(result["total_matches"], WORKSPACE_GLOB_MAX_MATCHES + 1);
}
