//! Trap 1: revenue joined through `customer_contacts` fan-out.
//!
//! The scripted model runs the naive join; the harness proves the trap is
//! real by showing the joined sum differs from the true sum. The assertion
//! is on the rows saya returned to the model, compared to a second
//! `saya query` that computes the truth without the join.

use super::common;

#[test]
fn duplicate_join_trap_is_visible() {
    let fixture = common::build_demo("join-trap");
    let inflated_sql = "SELECT COALESCE(SUM(o.amount_cents), 0) AS revenue \
        FROM orders o JOIN customer_contacts cc \
        ON cc.customer_id = o.customer_id";
    let query_args = serde_json::json!({ "sql": inflated_sql });
    let mut mock = common::spawn_mock(vec![
        common::tool_call_body("call_join", "bounded_sql_query", query_args),
        common::text_body("revenue reported"),
    ]);
    let output = common::run_ask(&fixture, mock.address(), "total revenue by region");
    mock.join();
    assert_eq!(
        output.status.code(),
        Some(0),
        "ask must answer: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    let bodies = mock.bodies();
    assert_eq!(bodies.len(), 2, "one tool round plus the answer round");
    let seen = common::tool_result_rows(&bodies[1]);
    assert_eq!(seen.len(), 1, "the joined query returns one row");
    let inflated = seen[0][0].as_i64().unwrap();
    let truth = common::run_query(
        &fixture,
        "SELECT COALESCE(SUM(amount_cents), 0) FROM orders",
    );
    let expected = truth[0][0].as_i64().unwrap();
    assert_ne!(
        inflated, expected,
        "the join must inflate revenue: joined {inflated} vs true {expected}",
    );
    let _ = std::fs::remove_dir_all(&fixture.root);
}
