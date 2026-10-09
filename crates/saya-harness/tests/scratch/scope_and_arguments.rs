use super::*;

#[cfg(unix)]
#[test]
fn the_created_scratch_file_is_0600() {
    use std::os::unix::fs::PermissionsExt;
    let run = TempRun::new("file-mode");
    assert!(!run.scratch_file().exists());
    let _tool = admitted(&run);
    let mode = fs::metadata(run.scratch_file())
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(
        mode, 0o600,
        "DuckDB creates 0644; the engine does not rely on the run directory alone"
    );
}

#[test]
fn the_tool_is_hidden_until_the_scratch_scope_is_approved() {
    let run = TempRun::new("admission");
    assert!(
        ScratchSql::definitions(&Capabilities::default()).is_empty(),
        "without the scope the tool is hidden, never advertised as always-empty"
    );
    assert!(
        ScratchSql::admit(run.root(), &Capabilities::default())
            .unwrap()
            .is_none(),
        "without the scope the tool does not exist"
    );
    assert!(
        !run.scratch_file().exists(),
        "a refused admission opens nothing"
    );

    let caps = scratch_approved();
    let definitions = ScratchSql::definitions(&caps);
    assert_eq!(definitions.len(), 1);
    let definition = &definitions[0];
    assert_eq!(definition.name, "scratch_sql");
    assert!(!definition.read_only, "the write is the honest surface");
    assert!(
        !definition.effect.requires_approval,
        "the write is plan-gated, not per-call approved"
    );
    assert_eq!(
        definition.effect.local_state,
        LocalStateEffect::WriteWorkspace,
        "the declared effect is write-shaped local state"
    );

    let _tool = ScratchSql::admit(run.root(), &caps).unwrap().unwrap();
    assert!(run.scratch_file().exists(), "the approved admission opens");
}

#[test]
fn the_tool_executor_refuses_bad_arguments_with_typed_errors() {
    let run = TempRun::new("executor-args");
    let tool = admitted(&run);
    let runner = |name: &str, args: Value| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(tool.execute(name, args))
    };
    assert!(matches!(
        runner("other", json!({"sql": "SELECT 1"})),
        Err(ToolError::UnsupportedTool)
    ));
    assert!(matches!(
        runner("scratch_sql", json!("SELECT 1")),
        Err(ToolError::ArgumentsNotObject)
    ));
    assert!(matches!(
        runner("scratch_sql", json!({})),
        Err(ToolError::UnsupportedProperty)
    ));
    assert!(matches!(
        runner("scratch_sql", json!({"sql": "SELECT 1", "extra": 1})),
        Err(ToolError::UnsupportedProperty)
    ));
    assert!(matches!(
        runner("scratch_sql", json!({"sql": 7})),
        Err(ToolError::SqlNotString)
    ));
    let ok = runner("scratch_sql", json!({"sql": "SELECT 42 AS answer"})).unwrap();
    assert_eq!(ok["rows"], json!([[42]]));
    let refused = runner("scratch_sql", json!({"sql": "INSTALL httpfs"})).unwrap_err();
    assert!(
        matches!(refused, ToolError::QueryFailedDetail(_)),
        "a refusal surfaces through the tool boundary: {refused}"
    );
    assert!(refused.to_string().contains("INSTALL httpfs"));
}
