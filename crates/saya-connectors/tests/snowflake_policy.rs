use saya_connectors::prepare_snowflake_sql;

#[test]
fn snowflake_policy_allows_read_features_and_caps_outer_query() {
    for sql in [
        "SELECT src:v FROM events QUALIFY row_number() OVER (ORDER BY id) = 1",
        "SHOW TABLES",
        "VALUES (1)",
    ] {
        assert!(prepare_snowflake_sql(sql, 3).is_ok(), "{sql}");
    }
    assert!(
        prepare_snowflake_sql("SELECT 1", 3)
            .unwrap()
            .contains("LIMIT 4")
    );
}

#[test]
fn snowflake_policy_denies_writes_stages_and_system_functions() {
    for sql in [
        "DELETE FROM events",
        "SELECT * FROM @stage/file",
        "SELECT SYSTEM$ABORT_SESSION()",
        "SELECT read_csv('x')",
        "SELECT 1; SELECT 2",
    ] {
        assert!(prepare_snowflake_sql(sql, 3).is_err(), "{sql}");
    }
}
