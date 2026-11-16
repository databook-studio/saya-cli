use super::*;

#[test]
fn test_sqlite_denied_functions() {
    assert!(prepare_sqlite_sql("SELECT load_extension('x')", 10).is_err());
    assert!(prepare_sqlite_sql("SELECT readfile('x')", 10).is_err());
    assert!(prepare_sqlite_sql("SELECT writefile('a', 'b')", 10).is_err());

    assert!(prepare_postgres_sql("SELECT load_extension('x')", 10).is_ok());
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
