use saya_connectors::{
    prepare_duckdb_sql, prepare_mysql_sql, prepare_postgres_sql, prepare_snowflake_sql,
};

#[path = "read_only_contract/read_cases.rs"]
mod read_cases;
#[path = "read_only_contract/rejection_cases.rs"]
mod rejection_cases;
