use sqlparser::dialect::{PostgreSqlDialect, SQLiteDialect};

use super::oracle::is_read_only_statement_class;

#[test]
fn test_independent_oracle_classification() {
    let pg = PostgreSqlDialect {};
    let sqlite = SQLiteDialect {};

    // Positive read-only cases
    assert!(is_read_only_statement_class("SELECT 1", &pg));
    assert!(is_read_only_statement_class(
        "SELECT a FROM b WHERE c = 1",
        &sqlite
    ));
    assert!(is_read_only_statement_class("VALUES (1), (2)", &pg));
    assert!(is_read_only_statement_class(
        "WITH cte AS (SELECT 1) SELECT * FROM cte",
        &sqlite
    ));
    assert!(is_read_only_statement_class("EXPLAIN SELECT 1", &pg));
    assert!(is_read_only_statement_class("SHOW TABLES", &pg));

    // Negative non-read-only cases
    assert!(!is_read_only_statement_class(
        "INSERT INTO t VALUES (1)",
        &pg
    ));
    assert!(!is_read_only_statement_class("UPDATE t SET x = 1", &sqlite));
    assert!(!is_read_only_statement_class("DELETE FROM t", &pg));
    assert!(!is_read_only_statement_class("DROP TABLE t", &sqlite));
    assert!(!is_read_only_statement_class("CREATE TABLE t (x INT)", &pg));
    assert!(!is_read_only_statement_class(
        "ALTER TABLE t ADD COLUMN x INT",
        &sqlite
    ));
    assert!(!is_read_only_statement_class("TRUNCATE TABLE t", &pg));
    assert!(!is_read_only_statement_class("COPY t FROM 'f'", &pg));
    assert!(!is_read_only_statement_class(
        "PRAGMA user_version",
        &sqlite
    ));
    assert!(!is_read_only_statement_class("SET timezone = 'UTC'", &pg));
    assert!(!is_read_only_statement_class(
        "ATTACH DATABASE 'a' AS b",
        &sqlite
    ));
    assert!(!is_read_only_statement_class("EXPLAIN DELETE FROM t", &pg));

    // Multiple statements or invalid SQL
    assert!(!is_read_only_statement_class("SELECT 1; SELECT 2", &pg));
    assert!(!is_read_only_statement_class(
        "INVALID SQL STATEMENT",
        &sqlite
    ));
}
