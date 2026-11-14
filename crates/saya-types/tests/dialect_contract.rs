//! The dialect surface is a closed list. DuckDB was removed, so no dialect
//! spells it, serde cannot revive it, and the list every caller sees is the
//! supported set.

use saya_types::SqlDialect;

#[test]
fn the_dialect_list_has_no_duckdb() {
    for spelling in ["duckdb", "duck_db"] {
        assert!(
            serde_json::from_str::<SqlDialect>(&format!("\"{spelling}\"")).is_err(),
            "DuckDB was removed, but {spelling:?} still deserializes"
        );
    }

    let listed = [
        SqlDialect::Postgres,
        SqlDialect::Mysql,
        SqlDialect::Snowflake,
        SqlDialect::Sqlite,
        SqlDialect::ClickHouse,
        SqlDialect::BigQuery,
    ]
    .map(SqlDialect::as_str);
    assert_eq!(
        listed,
        [
            "postgresql",
            "mysql",
            "snowflake",
            "sqlite",
            "clickhouse",
            "bigquery",
        ],
        "the supported dialect list"
    );
}
