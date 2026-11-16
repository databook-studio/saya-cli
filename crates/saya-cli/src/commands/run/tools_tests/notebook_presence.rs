/// A run step's `notebook_edit` is decided by the run's workspace: the tool is
/// offered only where the tree holds a `.ipynb`, so a run without one
/// advertises the list it carried before the tool existed — same names, same
/// order — and refuses a call to it as an unknown tool.
#[tokio::test]
async fn a_run_without_a_notebook_advertises_no_notebook_edit() {
    let database = Arc::new(DatabaseTools::new(None, 100, true));
    let workspace = workspace("notebook-absent");
    let mut write_scopes = Capabilities::default();
    write_scopes.workspace_write = true;
    let built = toolsets(
        ToolsetInputs {
            database: &database,
            fetch: None,
            shell: None,
            workspace: &workspace,
            allow_query_data: true,
            tools: &saya_config::ResolvedTools::default(),
            mcp: None,
            hooks: None,
        },
        &[step("write files", write_scopes)],
    );
    let advertised = names(&built, 0);
    assert_eq!(
        advertised,
        [
            "schema_discovery",
            "request_clarification",
            "read",
            "glob",
            "grep",
            "edit",
            "write",
            "bounded_sql_query",
            "bounded_sql_query_all",
            "result_shape",
            "column_health",
            "join_check",
            "designate_answer",
        ],
        "a run without a notebook advertises the pre-P11 list, name for name"
    );
    let refused = built[0]
        .executor
        .execute(
            "notebook_edit",
            serde_json::json!({"path": "analysis.ipynb", "mode": "delete", "index": 0}),
        )
        .await
        .expect_err("a name this step's composition dropped must refuse");
    assert!(
        matches!(refused, saya_agent::ToolError::UnsupportedTool),
        "the refusal is the unknown-tool one: {refused:?}"
    );
}

/// The same step over a workspace that holds one: the edit joins the list,
/// last, after every tool that existed before it.
#[tokio::test]
async fn a_run_with_a_notebook_appends_the_notebook_edit_last() {
    let database = Arc::new(DatabaseTools::new(None, 100, true));
    let workspace = workspace("notebook-present");
    fs::write(workspace.root().join("analysis.ipynb"), b"{\"cells\":[]}").unwrap();
    let mut write_scopes = Capabilities::default();
    write_scopes.workspace_write = true;
    let built = toolsets(
        ToolsetInputs {
            database: &database,
            fetch: None,
            shell: None,
            workspace: &workspace,
            allow_query_data: true,
            tools: &saya_config::ResolvedTools::default(),
            mcp: None,
            hooks: None,
        },
        &[step("write files", write_scopes)],
    );
    let advertised = names(&built, 0);
    assert_eq!(
        advertised.last().map(String::as_str),
        Some("notebook_edit"),
        "the notebook edit goes last: {advertised:?}"
    );
    assert_eq!(
        advertised.len(),
        14,
        "it is appended, not inserted: {advertised:?}"
    );
}
