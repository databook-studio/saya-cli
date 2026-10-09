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
        .execute("workspace_list", serde_json::json!({}))
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
        .execute("workspace_list", serde_json::json!({"path": "notes"}))
        .await
        .expect("listing a subdirectory must succeed");
    assert_eq!(notes["path"], "notes");
    assert_eq!(notes["entries"][0]["name"], "summary.md");
    assert_eq!(notes["entries"][0]["kind"], "file");
    assert_eq!(notes["entries"][0]["size"].as_u64(), Some(15));
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
        .execute("workspace_list", serde_json::json!({}))
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
            .execute("workspace_list", serde_json::json!({"path": path}))
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

/// A directory holding more than the entry bound is a typed bounds error, not
/// a quietly truncated list.
#[tokio::test]
async fn workspace_list_reports_an_entry_count_overrun_as_a_typed_error() {
    let sandbox = Sandbox::new("list-bound");
    sandbox.plant(
        (0..WORKSPACE_LIST_MAX_ENTRIES + 1).map(|index| format!("f{index}.txt")),
        b"",
    );
    let tools = sandbox.tools();
    let error = tools
        .execute("workspace_list", serde_json::json!({}))
        .await
        .expect_err("an over-bound directory must be refused, not truncated");
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

#[tokio::test]
async fn workspace_list_rejects_a_non_string_path_and_unknown_arguments() {
    let tools = DatabaseTools::with_registry(
        crate::connection::ConnectionRegistry::new("primary"),
        100,
        true,
        None,
    );
    let error = tools
        .execute("workspace_list", serde_json::json!({"path": 7}))
        .await
        .expect_err("a non-string path must be rejected at validation");
    assert_eq!(error, ToolError::PathNotString);
    let error = tools
        .execute(
            "workspace_list",
            serde_json::json!({"path": "notes", "sql": "SELECT 1"}),
        )
        .await
        .expect_err("unknown arguments must be rejected");
    assert_eq!(error, ToolError::UnsupportedProperty);
}
