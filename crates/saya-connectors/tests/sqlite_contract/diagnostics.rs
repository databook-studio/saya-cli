use super::*;

/// SQLite ships `sqrt`, `pow`, `ceil`, `floor`, `mod`, the logarithms and the
/// trigonometric functions, but only when it is compiled with
/// `SQLITE_ENABLE_MATH_FUNCTIONS` — the bundled build does not enable it by
/// default. Without them any question involving a distance, a rate or a
/// rounding boundary fails against a SQLite profile while working against a
/// server engine, which is a difference the user never asked for.
#[tokio::test]
async fn sqlite_has_the_standard_maths_functions() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let path = temp_dir.path().join("maths.db");
    create_test_fixture(&path).await;

    let connector = SqliteConnector::open(&path, true, ConnectorOptions::default())
        .await
        .expect("fixture opens");
    connector.connect().await.expect("connect");

    for expression in [
        "sqrt(4.0)",
        "pow(2.0, 3.0)",
        "ceil(1.2)",
        "floor(1.8)",
        "mod(5, 2)",
        "exp(0.0)",
        "ln(1.0)",
        "log10(100.0)",
        "sin(0.0)",
        "cos(0.0)",
        "acos(1.0)",
        "asin(0.0)",
        "atan(0.0)",
        "atan2(0.0, 1.0)",
        "radians(180.0)",
        "degrees(0.0)",
        "pi()",
    ] {
        let req = QueryRequest {
            sql: format!("SELECT {expression} AS value"),
            max_rows: 1,
            params: Vec::new(),
        };
        let result = connector.execute(req).await;
        assert!(
            result.is_ok(),
            "`{expression}` must be available to a SQLite profile, got {:?}",
            result.err()
        );
    }
}

/// A query can fail because the SQL names something that is not there, and the
/// agent writing that SQL is the one who has to fix it. Reporting only "SQLite
/// query failed" leaves it guessing: a missing function, a misspelt table and a
/// syntax error are indistinguishable, so it retries blind and burns its turn
/// budget probing.
///
/// The identifiers echoed here come from the SQL the caller just wrote, not
/// from any row, so naming them discloses nothing the caller did not already
/// have. Anything the classifier does not recognise stays redacted.
#[tokio::test]
async fn a_failing_query_says_what_the_sql_got_wrong() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let path = temp_dir.path().join("errors.db");
    create_test_fixture(&path).await;

    let connector = SqliteConnector::open(&path, true, ConnectorOptions::default())
        .await
        .expect("fixture opens");
    connector.connect().await.expect("connect");

    for (sql, expected) in [
        ("SELECT no_such_fn(1) AS v", "no_such_fn"),
        ("SELECT * FROM not_a_table", "not_a_table"),
        ("SELECT not_a_column FROM t", "not_a_column"),
    ] {
        let req = QueryRequest {
            sql: sql.to_string(),
            max_rows: 1,
            params: Vec::new(),
        };
        let err = connector
            .execute(req)
            .await
            .expect_err("the query must fail");
        let text = err.to_string();
        assert!(
            text.contains(expected),
            "the error must name `{expected}` so the caller can correct the SQL, got: {text}"
        );
    }
}

/// The classifier is an allowlist, not a pass-through: a failure it does not
/// recognise keeps the redacted wording, so a driver message that might carry
/// row values cannot reach the user through this path.
#[tokio::test]
async fn an_unrecognised_failure_stays_redacted() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let path = temp_dir.path().join("redacted.db");
    create_test_fixture(&path).await;

    let connector = SqliteConnector::open(&path, true, ConnectorOptions::default())
        .await
        .expect("fixture opens");
    connector.connect().await.expect("connect");

    // A write against a read-only connection: refused for a reason that is not
    // about a name in the SQL, so nothing is echoed back.
    let req = QueryRequest {
        sql: "INSERT INTO t (id) VALUES (99)".to_string(),
        max_rows: 1,
        params: Vec::new(),
    };
    let err = connector
        .execute(req)
        .await
        .expect_err("a write must fail on a read-only connection");
    let text = err.to_string();
    assert!(
        !text.contains("99"),
        "an unrecognised failure must not echo query content: {text}"
    );
}
