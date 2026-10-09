#[tokio::test]
async fn glob_rejects_a_non_string_pattern_and_unknown_arguments() {
    let tools = DatabaseTools::with_registry(
        crate::connection::ConnectionRegistry::new("primary"),
        100,
        true,
        None,
    );
    let error = tools
        .execute("glob", serde_json::json!({"pattern": 7}))
        .await
        .expect_err("a non-string pattern must be rejected at validation");
    assert_eq!(error, ToolError::PatternNotString);
    let error = tools
        .execute(
            "glob",
            serde_json::json!({"pattern": "*.md", "path": "notes"}),
        )
        .await
        .expect_err("unknown arguments must be rejected");
    assert_eq!(error, ToolError::UnsupportedProperty);
}
