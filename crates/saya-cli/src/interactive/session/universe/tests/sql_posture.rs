// ---------------------------------------------------------------------------
// the SQL safety layer is untouchable under bypass
// ---------------------------------------------------------------------------


/// The SQL safety layer is not an approval question, so bypass does not move
/// it (DESIGN §4, test 14): a write statement through the query tools still
/// refuses under a bypass policy — and the refusal is the safety layer's own
/// words, not the approval engine's, because the engine resolved `Allow` and
/// handed the call to the tool. A real SQLite connector backs the registry,
/// so the statement meets the actual `prepare_*` gate.
#[tokio::test]
async fn bypass_leaves_the_sql_safety_layer_untouched() {
    use saya_connectors::{ConnectorOptions, SqliteConnector};
    let project = worktree("safety-layer");
    let state = temp_dir("safety-layer-state");
    let universe = compose(&session_runtime(), &project, &state);
    let database_path = state.join("safety.sqlite");
    std::fs::File::create(&database_path).expect("the sqlite file is created");
    let connector = SqliteConnector::open(&database_path, false, ConnectorOptions::default())
        .await
        .expect("the sqlite file opens");
    let database = Arc::new(crate::agent::tools::DatabaseTools::new(
        Some(Box::new(connector)),
        100,
        true,
    ));
    // The engine half: under bypass the write-shaped SQL call is *allowed* —
    // the refusal cannot come from approval.
    let sql = universe
        .definitions(
            saya_agent::AgentMode::Build,
            false,
            ApprovalPolicy::Bypass,
            false,
            true,
            false,
            false,
        
            &Default::default(),
        )
        .iter()
        .find(|definition| definition.name == "bounded_sql_query")
        .expect("the read-shaped SQL tools are always advertised")
        .clone();
    assert_eq!(
        saya_agent::SessionPolicy::new(ApprovalPolicy::Bypass).resolve(&sql.effect, None),
        saya_agent::ApprovalDecision::Allow,
        "bypass allows the SQL call: the safety layer is the next line of defence, not approval"
    );
    let executor = universe.executor(
        database,
        &CancellationToken::new(),
        None,
        None,
        None,
        Default::default(),
    );
    let refused = executor
        .execute(
            "bounded_sql_query",
            serde_json::json!({"sql": "DROP TABLE users"}),
        )
        .await
        .expect_err("a write statement still refuses under bypass");
    let refusal = format!("{refused:?}");
    assert!(
        refusal.contains("read-only safety policy")
            || refusal.contains("not parseable as one read-only statement"),
        "the refusal is the safety layer's own: {refusal}"
    );
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}
