mod for_dialect;
mod params;
mod read_only;
mod read_only_policy;
mod references;
mod reject;

#[cfg(test)]
mod for_dialect_tests;

#[cfg(test)]
mod params_tests;

pub use for_dialect::prepare_for_dialect;
pub use params::{PreparedQuery, prepare_with_params, sql_placeholders};
pub(crate) use read_only::parser_dialect;
pub use read_only::{
    prepare_bigquery_sql, prepare_clickhouse_sql, prepare_duckdb_sql, prepare_mysql_sql,
    prepare_postgres_sql, prepare_snowflake_sql, prepare_sqlite_sql,
};
pub use references::{SqlReferences, sql_references};
