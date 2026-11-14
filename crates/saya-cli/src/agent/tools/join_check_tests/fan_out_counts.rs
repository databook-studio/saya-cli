//! What `join_check` reports about row counts: a fanned-out join is flagged,
//! a clean 1:1 join is cleared, a row-dropping join is reported, and a
//! statement with no sound probe returns `applicable: false` with a reason.

use super::*;

use saya_agent::ToolExecutor;

#[tokio::test]
async fn join_check_flags_a_fanned_out_join() {
    let tools = DatabaseTools::new(
        Some(Box::new(CountConnector {
            joined: 100,
            base: 50,
        })),
        100,
        true,
    );
    let result = tools
        .execute("join_check", serde_json::json!({"sql": JOIN_SQL}))
        .await
        .expect("join_check should succeed on a join with an aggregate");

    assert_eq!(result["applicable"], true);
    assert_eq!(result["joined_rows"], 100);
    assert_eq!(result["base_rows"], 50);
    assert_eq!(result["fanned_out"], true);
    assert_eq!(result["dropped_rows"], false);
}

#[tokio::test]
async fn join_check_clears_a_clean_one_to_one_join() {
    let tools = DatabaseTools::new(
        Some(Box::new(CountConnector {
            joined: 50,
            base: 50,
        })),
        100,
        true,
    );
    let result = tools
        .execute("join_check", serde_json::json!({"sql": JOIN_SQL}))
        .await
        .expect("join_check should succeed");

    assert_eq!(result["applicable"], true);
    assert_eq!(result["joined_rows"], 50);
    assert_eq!(result["base_rows"], 50);
    assert_eq!(result["fanned_out"], false);
    assert_eq!(result["dropped_rows"], false);
}

#[tokio::test]
async fn join_check_reports_a_row_dropping_join() {
    let tools = DatabaseTools::new(
        Some(Box::new(CountConnector {
            joined: 30,
            base: 50,
        })),
        100,
        true,
    );
    let result = tools
        .execute("join_check", serde_json::json!({"sql": JOIN_SQL}))
        .await
        .expect("join_check should succeed");

    assert_eq!(result["applicable"], true);
    assert_eq!(result["joined_rows"], 30);
    assert_eq!(result["base_rows"], 50);
    assert_eq!(result["fanned_out"], false);
    assert_eq!(result["dropped_rows"], true);
}

#[tokio::test]
async fn join_check_returns_not_applicable_when_no_sound_probe_can_be_built() {
    let tools = DatabaseTools::new(
        Some(Box::new(CountConnector { joined: 0, base: 0 })),
        100,
        true,
    );
    let result = tools
        .execute(
            "join_check",
            serde_json::json!({"sql": "SELECT SUM(x) FROM single_table"}),
        )
        .await
        .expect("join_check should succeed with applicable: false");

    assert_eq!(result["applicable"], false);
    assert!(
        result["reason"].as_str().is_some_and(|r| !r.is_empty()),
        "a reason must accompany applicable: false"
    );
}
