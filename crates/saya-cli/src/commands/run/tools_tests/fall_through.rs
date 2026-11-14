/// The composite dispatches the fixed harness names — and refuses the
/// ones no member backs exactly as the single executor does: the database
/// tools' typed `UnsupportedTool`, fail closed. `run_program` is on this
/// list: its member is wired, and its narrowing is pinned in the runner
/// tests below.
#[tokio::test]
async fn the_harness_names_fall_through_to_the_same_typed_refusal() {
    let database = Arc::new(DatabaseTools::new(None, 100, true));
    let workspace = workspace("fall-through");
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
        &[step("read only", Capabilities::default())],
    );
    for name in ["http_fetch", "http_download", "wat"] {
        let error = built[0]
            .executor
            .execute(name, serde_json::json!({}))
            .await
            .expect_err("nothing in this slice can run any of these names");
        assert_eq!(
            error,
            ToolError::UnsupportedTool,
            "`{name}` must be refused with today's typed error, got: {error:?}"
        );
    }
}
