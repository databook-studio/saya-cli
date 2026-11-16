#[tokio::test]
async fn workspace_list_returns_sorted_entries_with_kinds_and_sizes() {
    let sandbox = Sandbox::new("list-content");
    sandbox
        .ws
        .write("notes/summary.md", b"hello workspace")
        .expect("contained write must succeed");
    sandbox
        .ws
        .write("top.txt", b"abcde")
        .expect("contained write must succeed");
    let tools = sandbox.tools();
    let root = tools
        .execute("list", serde_json::json!({}))
        .await
        .expect("listing the root must succeed");
    assert_eq!(root["path"], "");
    let entries = root["entries"].as_array().expect("entries array");
    let names: Vec<&str> = entries
        .iter()
        .map(|entry| entry["name"].as_str().expect("name"))
        .collect();
    assert_eq!(
        names,
        vec!["notes", "top.txt"],
        "entries are sorted by name"
    );
    assert_eq!(entries[0]["kind"], "dir");
    assert_eq!(entries[1]["kind"], "file");
    assert_eq!(entries[1]["size"].as_u64(), Some(5));
    let notes = tools
        .execute("list", serde_json::json!({"path": "notes"}))
        .await
        .expect("listing a subdirectory must succeed");
    assert_eq!(notes["path"], "notes");
    assert_eq!(notes["entries"][0]["name"], "summary.md");
    assert_eq!(notes["entries"][0]["kind"], "file");
    assert_eq!(notes["entries"][0]["size"].as_u64(), Some(15));
    for path in ["./notes", "notes/"] {
        let listed = tools
            .execute("list", serde_json::json!({"path": path}))
            .await
            .unwrap_or_else(|error| panic!("listing {path:?} must succeed: {error}"));
        assert_eq!(listed["entries"], notes["entries"], "{path}");
    }
    for path in [".", "./"] {
        let listed = tools
            .execute("list", serde_json::json!({"path": path}))
            .await
            .unwrap_or_else(|error| panic!("listing {path:?} must list the workspace root: {error}"));
        assert_eq!(listed["entries"], root["entries"], "{path}");
    }
}

/// No workspace attached (every current caller, until the run engine opens
/// one): the tool denies with a typed error instead of pretending to list.
#[tokio::test]
async fn workspace_list_denies_when_no_workspace_is_attached() {
    let tools = DatabaseTools::with_registry(
        crate::connection::ConnectionRegistry::new("primary"),
        100,
        true,
        None,
    );
    let error = tools
        .execute("list", serde_json::json!({}))
        .await
        .expect_err("no workspace, no listing");
    assert_eq!(error, ToolError::WorkspaceUnavailable);
}

/// The escape the containment layer exists for: a `..` path that rises above
/// the root is refused as a typed `ToolError` carrying the harness's own
/// reason — readable by the model, not a panic and not a silent empty list.
#[tokio::test]
async fn workspace_list_surfaces_a_path_escape_as_a_typed_error() {
    let sandbox = Sandbox::new("list-escape");
    fs::write(sandbox.outer.join("outside.txt"), b"secret").expect("plant must succeed");
    let tools = sandbox.tools();
    for path in ["../outside.txt", "/etc/passwd"] {
        let error = tools
            .execute("list", serde_json::json!({"path": path}))
            .await
            .expect_err("an escaping path must be refused, not listed");
        match error {
            ToolError::Workspace(message) => {
                assert!(
                    message.contains("escapes the run workspace"),
                    "the refusal must name the containment failure: {message}"
                );
                assert!(
                    message.contains(path),
                    "the refusal must name the refused path: {message}"
                );
            }
            other => panic!("expected a typed workspace error, got: {other}"),
        }
    }
}

/// A directory holding more than the entry bound is served short with an
/// omitted count, never refused: the model learns the directory is larger
/// without losing the call and guessing at how to narrow.
#[tokio::test]
async fn workspace_list_truncates_an_entry_count_overrun_with_an_omitted_count() {
    let sandbox = Sandbox::new("list-bound");
    sandbox.plant((0..600).map(|index| format!("f{index:03}.txt")), b"");
    let tools = sandbox.tools();
    let result = tools
        .execute("list", serde_json::json!({}))
        .await
        .expect("an over-bound directory must list, not refuse");
    let entries = result["entries"].as_array().expect("entries array");
    assert_eq!(entries.len(), WORKSPACE_LIST_MAX_ENTRIES);
    assert_eq!(entries[0]["name"], "f000.txt");
    assert_eq!(entries[entries.len() - 1]["name"], "f499.txt");
    assert_eq!(result["truncated"], true);
    assert_eq!(result["omitted"], 100);
    assert!(
        result.get("omitted_exact").is_none(),
        "an exact count is the default and is not carried: {result}"
    );
}

/// The caller's own result cap bounds the listing too, and is kept the same
/// way: entries it cannot hold join the omitted count instead of failing the
/// call.
#[tokio::test]
async fn workspace_list_serves_short_when_the_caller_cap_cannot_hold_every_entry() {
    let sandbox = Sandbox::new("list-caller-cap");
    sandbox.plant((0..100).map(|index| format!("f{index:03}.txt")), b"");
    let tools = sandbox.tools();
    let result = tools
        .execute_with_context(
            "list",
            serde_json::json!({}),
            saya_agent::ToolExecutionContext {
                result_cap: 1024,
                cancellation: saya_agent::CancellationToken::new(),
            },
        )
        .await
        .expect("a listing too large for the caller cap must be served short");
    assert!(
        serde_json::to_vec(&result).expect("serializable").len() <= 1024,
        "the served listing must fit the caller's cap"
    );
    let entries = result["entries"].as_array().expect("entries array");
    assert!(!entries.is_empty(), "a capped listing must keep real entries");
    assert!(entries.len() < 100);
    assert_eq!(result["truncated"], true);
    assert_eq!(
        result["omitted"].as_u64(),
        Some((100 - entries.len()) as u64),
        "every dropped entry joins the omitted count"
    );
}

#[tokio::test]
async fn workspace_list_rejects_a_non_string_path_and_unknown_arguments() {
    let tools = DatabaseTools::with_registry(
        crate::connection::ConnectionRegistry::new("primary"),
        100,
        true,
        None,
    );
    let error = tools
        .execute("list", serde_json::json!({"path": 7}))
        .await
        .expect_err("a non-string path must be rejected at validation");
    assert_eq!(error, ToolError::PathNotString);
    let error = tools
        .execute(
            "list",
            serde_json::json!({"path": "notes", "sql": "SELECT 1"}),
        )
        .await
        .expect_err("unknown arguments must be rejected");
    assert_eq!(error, ToolError::UnsupportedProperty);
}
