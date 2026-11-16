//! The privacy invariant: no cell value the probe reads ever appears in the
//! serialized join_check return value.

use super::*;

use saya_agent::ToolExecutor;

#[tokio::test]
async fn join_check_never_leaks_a_planted_cell_value() {
    let tools = DatabaseTools::new(Some(Box::new(SentinelCountConnector)), 100, true);
    let result = tools
        .execute("join_check", serde_json::json!({"sql": JOIN_SQL}))
        .await
        .expect("join_check should succeed (returning applicable: false)");

    let serialized = serde_json::to_string(&result).expect("result serializes");
    assert!(
        !serialized.contains("JOIN_CHECK_SENTINEL_77"),
        "a cell value escaped into the join_check output: {serialized}"
    );
}
