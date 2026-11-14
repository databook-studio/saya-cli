//! Trap 2: the time column changes the answer.
//!
//! Two scripted runs ask "orders since 2026-01-01": one filters on
//! `orders.order_date`, the other on `customers.signup_date`. Both counts
//! must equal their independently computed truths, and the two truths must
//! differ — the column choice is material, not cosmetic.

use super::common;

/// Runs one ask round with a scripted COUNT query; returns the count the
/// model saw plus the ask's exit status.
fn scripted_count(fixture: &common::DemoFixture, sql: &str) -> (i64, Option<i32>) {
    let args = serde_json::json!({ "sql": sql });
    let mut mock = common::spawn_mock(vec![
        common::tool_call_body("call_count", "bounded_sql_query", args),
        common::text_body("count reported"),
    ]);
    let output = common::run_ask(fixture, mock.address(), "orders since 2026-01-01");
    mock.join();
    let status = output.status.code();
    assert_eq!(
        status,
        Some(0),
        "ask must answer: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    let bodies = mock.bodies();
    assert_eq!(bodies.len(), 2);
    let seen = common::tool_result_rows(&bodies[1]);
    assert_eq!(seen.len(), 1);
    (seen[0][0].as_i64().unwrap(), status)
}

#[test]
fn time_column_choice_changes_the_answer() {
    let fixture = common::build_demo("time-column");
    let by_order_sql = "SELECT COUNT(*) FROM orders WHERE order_date >= '2026-01-01'";
    let by_signup_sql = "SELECT COUNT(*) FROM orders o JOIN customers c \
        ON c.id = o.customer_id WHERE c.signup_date >= '2026-01-01'";
    let (by_order, _) = scripted_count(&fixture, by_order_sql);
    let (by_signup, _) = scripted_count(&fixture, by_signup_sql);
    let truth_order = common::run_query(&fixture, by_order_sql);
    let truth_signup = common::run_query(&fixture, by_signup_sql);
    let expected_order = truth_order[0][0].as_i64().unwrap();
    let expected_signup = truth_signup[0][0].as_i64().unwrap();
    assert_eq!(by_order, expected_order, "order_date count is exact");
    assert_eq!(by_signup, expected_signup, "signup_date count is exact");
    assert_ne!(
        expected_order, expected_signup,
        "the column choice must change the answer",
    );
    let _ = std::fs::remove_dir_all(&fixture.root);
}
