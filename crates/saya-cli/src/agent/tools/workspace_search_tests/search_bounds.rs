/// Both search tools share one visited-entries discipline: walking past the
/// visited bound is a typed error for each of them, never a quiet stop.
#[tokio::test]
async fn search_reports_a_visited_count_overrun_as_a_typed_error() {
    let sandbox = Sandbox::new("search-visited-bound");
    let total = WORKSPACE_GLOB_MAX_VISITED.max(WORKSPACE_GREP_MAX_VISITED) + 1;
    sandbox.plant((0..total).map(|index| format!("f{index}.txt")), b"");
    let tools = sandbox.tools();
    let calls = [
        ("glob", serde_json::json!({"pattern": "*.txt"})),
        (
            "grep",
            serde_json::json!({"pattern": "needle", "case_insensitive": false}),
        ),
    ];
    for (name, arguments) in calls {
        let error = tools
            .execute(name, arguments)
            .await
            .expect_err("an over-bound walk must be refused, not truncated");
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
}

/// A symlink into the outside world is a decoy twice over: `glob` must not
/// match it by name (`decoy.md` matches `*.md`), must not descend into a
/// symlinked directory, and `grep` must never read through either.
#[cfg(unix)]
#[tokio::test]
async fn search_neither_follows_nor_matches_an_outside_symlink() {
    use std::os::unix::fs::symlink;

    let sandbox = Sandbox::new("symlink");
    fs::write(sandbox.outer.join("outside.md"), b"outside needle")
        .expect("outside plant must succeed");
    fs::create_dir_all(sandbox.outer.join("outside_dir")).expect("outside dir must create");
    fs::write(
        sandbox.outer.join("outside_dir").join("inside.md"),
        b"inside needle",
    )
    .expect("outside plant must succeed");
    symlink("../outside.md", sandbox.ws.root().join("decoy.md"))
        .expect("decoy symlink must create");
    symlink("../outside_dir", sandbox.ws.root().join("linkdir"))
        .expect("linkdir symlink must create");
    sandbox
        .ws
        .write("real/inner.md", b"real needle")
        .expect("contained write must succeed");
    let tools = sandbox.tools();
    let result = tools
        .execute("glob", serde_json::json!({"pattern": "**/*.md"}))
        .await
        .expect("a contained glob must succeed");
    let paths: Vec<String> = result["matches"]
        .as_array()
        .expect("matches array")
        .iter()
        .map(|value| value.as_str().expect("path string").to_string())
        .collect();
    assert_eq!(paths, vec!["real/inner.md"]);
    let result = tools
        .execute("grep", serde_json::json!({"pattern": "needle"}))
        .await
        .expect("a contained grep must succeed");
    let hits = result["matches"].as_array().expect("matches array");
    assert_eq!(hits.len(), 1, "only the real file is searched");
    assert_eq!(hits[0]["path"], "real/inner.md");
    assert_eq!(result["files_scanned"].as_u64(), Some(1));
    assert_eq!(result["files_skipped"].as_u64(), Some(0));
}
