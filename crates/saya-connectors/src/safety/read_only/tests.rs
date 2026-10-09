use super::*;

#[test]
fn test_sqlite_denied_functions() {
    assert!(prepare_sqlite_sql("SELECT load_extension('x')", 10).is_err());
    assert!(prepare_sqlite_sql("SELECT readfile('x')", 10).is_err());
    assert!(prepare_sqlite_sql("SELECT writefile('a', 'b')", 10).is_err());

    assert!(prepare_postgres_sql("SELECT load_extension('x')", 10).is_ok());
}

#[test]
fn test_duckdb_denied_functions() {
    assert!(prepare_duckdb_sql("SELECT * FROM read_csv('x')", 10).is_err());

    assert!(prepare_postgres_sql("SELECT * FROM read_csv('x')", 10).is_ok());
    assert!(prepare_sqlite_sql("SELECT * FROM read_csv('x')", 10).is_ok());
}

/// Parquet table functions reach outside a staged database file, so the
/// session's read-only DuckDB policy must refuse every spelling —
/// `read_parquet` and its aliases `parquet_scan`/`parquet_schema`, plus
/// the metadata readers. C2a stages Parquet into a private snapshot file;
/// a session over that snapshot must never be able to open other files.
#[test]
fn test_duckdb_denied_parquet_file_functions() {
    assert!(prepare_duckdb_sql("SELECT * FROM read_parquet('x.parquet')", 10).is_err());
    assert!(prepare_duckdb_sql("SELECT * FROM parquet_scan('x.parquet')", 10).is_err());
    assert!(prepare_duckdb_sql("SELECT * FROM parquet_metadata('x.parquet')", 10).is_err());
    assert!(prepare_duckdb_sql("SELECT * FROM parquet_schema('x.parquet')", 10).is_err());
    assert!(prepare_duckdb_sql("SELECT count(*) FROM parquet_scan('x.parquet')", 10).is_err());
    assert!(prepare_duckdb_sql("SELECT * FROM parquet_scan('x.parquet'), t", 10).is_err());
}

/// The DuckDB gate refuses every file/network-reading table function the
/// scratch validator refuses — the `read_`/`http_` prefix families plus
/// the prefixless readers (parquet, sqlite/postgres/mysql scanners, arrow,
/// iceberg, delta, glob, metadata). Each name must be refused wherever a
/// function or relation can appear, case-insensitively and
/// schema-qualified, while ordinary reads stay allowed.
#[test]
fn test_duckdb_denied_file_reading_functions() {
    const NAMES: &[&str] = &[
        "read_csv",
        "read_csv_auto",
        "read_json",
        "read_json_auto",
        "read_parquet",
        "read_text",
        "read_blob",
        "http_get",
        "parquet_scan",
        "parquet_metadata",
        "parquet_schema",
        "parquet_file_metadata",
        "parquet_kv_metadata",
        "sqlite_scan",
        "postgres_scan",
        "mysql_scan",
        "arrow",
        "arrow_scan",
        "iceberg_scan",
        "delta_scan",
        "glob",
        "metadata",
    ];
    for name in NAMES {
        let upper = name.to_ascii_uppercase();
        let qualified = format!("main.{name}('x')");
        let cases: [(&str, String); 7] = [
            ("FROM", format!("SELECT * FROM {name}('x')")),
            ("JOIN", format!("SELECT * FROM t, {name}('x')")),
            (
                "subquery",
                format!("SELECT * FROM (SELECT * FROM {name}('x')) s"),
            ),
            (
                "CTE",
                format!("WITH c AS (SELECT * FROM {name}('x')) SELECT * FROM c"),
            ),
            ("scalar", format!("SELECT {name}('x')")),
            ("qualified", format!("SELECT * FROM {qualified}")),
            ("upper", format!("SELECT * FROM {upper}('x')")),
        ];
        for (position, sql) in cases {
            assert!(
                prepare_duckdb_sql(&sql, 10).is_err(),
                "DuckDB gate must refuse {name} in {position}: {sql}"
            );
        }
    }
    assert!(prepare_duckdb_sql("SELECT * FROM t", 10).is_ok());
    assert!(prepare_duckdb_sql("SELECT length('x')", 10).is_ok());
    assert!(prepare_duckdb_sql("WITH c AS (SELECT 1 AS n) SELECT n FROM c", 10).is_ok());
}

#[test]
fn test_snowflake_denied_prefixes() {
    assert!(prepare_snowflake_sql("SELECT system$type('x')", 10).is_err());
    assert!(prepare_snowflake_sql("SELECT * FROM @stage", 10).is_err());

    assert!(prepare_postgres_sql("SELECT system$type('x')", 10).is_ok());
}

#[test]
fn test_snowflake_denied_functions() {
    assert!(prepare_snowflake_sql("SELECT GET_PRESIGNED_URL(@stage, 'secret.csv')", 10).is_err());
    assert!(
        prepare_snowflake_sql("SELECT BUILD_SCOPED_FILE_URL(@stage, 'secret.csv')", 10).is_err()
    );
    assert!(prepare_snowflake_sql("SELECT * FROM DIRECTORY(@stage)", 10).is_err());

    assert!(prepare_duckdb_sql("SELECT * FROM read_csv('x')", 10).is_err());
    assert!(prepare_postgres_sql("SELECT GET_PRESIGNED_URL('stage', 'secret.csv')", 10).is_ok());
}

#[test]
fn test_common_denied_functions() {
    assert!(prepare_postgres_sql("SELECT nextval('seq')", 10).is_err());
    assert!(prepare_postgres_sql("SELECT setval('seq', 1)", 10).is_err());
    assert!(prepare_sqlite_sql("SELECT nextval('seq')", 10).is_err());
    assert!(prepare_sqlite_sql("SELECT setval('seq', 1)", 10).is_err());
}

#[test]
fn test_sqlite_prepare() {
    let sql = prepare_sqlite_sql("SELECT 1 FROM t", 10).unwrap();
    assert!(sql.to_uppercase().contains("LIMIT 11"));
}
