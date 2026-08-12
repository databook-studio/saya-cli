use saya_types::ConnectionError;
use std::ops::ControlFlow;

use sqlparser::{
    ast::{Expr, ObjectName, Query, SetExpr, Statement, Visit, Visitor},
    dialect::{
        Dialect, DuckDbDialect, MySqlDialect, PostgreSqlDialect, SQLiteDialect, SnowflakeDialect,
    },
    parser::Parser,
};

struct BackendPolicy {
    denied_functions: &'static [&'static str],
    denied_prefixes: &'static [&'static str],
}

const COMMON_DENIED_FUNCTIONS: &[&str] = &["nextval", "setval"];

const DUCKDB_DENIED_FUNCTIONS: &[&str] = &[
    "read_csv",
    "read_csv_auto",
    "read_json",
    "read_json_auto",
    "read_parquet",
    "read_text",
    "sqlite_scan",
    "glob",
    "metadata",
];

const SQLITE_DENIED_FUNCTIONS: &[&str] = &["load_extension", "readfile", "writefile"];

const SNOWFLAKE_DENIED_FUNCTIONS: &[&str] =
    &["get_presigned_url", "build_scoped_file_url", "directory"];

const SNOWFLAKE_DENIED_PREFIXES: &[&str] = &["@", "system$"];

const POSTGRES_POLICY: BackendPolicy = BackendPolicy {
    denied_functions: &[],
    denied_prefixes: &[],
};

const MYSQL_POLICY: BackendPolicy = BackendPolicy {
    denied_functions: &[],
    denied_prefixes: &[],
};

const DUCKDB_POLICY: BackendPolicy = BackendPolicy {
    denied_functions: DUCKDB_DENIED_FUNCTIONS,
    denied_prefixes: &[],
};

const SQLITE_POLICY: BackendPolicy = BackendPolicy {
    denied_functions: SQLITE_DENIED_FUNCTIONS,
    denied_prefixes: &[],
};

const SNOWFLAKE_POLICY: BackendPolicy = BackendPolicy {
    denied_functions: SNOWFLAKE_DENIED_FUNCTIONS,
    denied_prefixes: SNOWFLAKE_DENIED_PREFIXES,
};

pub fn prepare_postgres_sql(sql: &str, max_rows: usize) -> Result<String, ConnectionError> {
    prepare(sql, max_rows, &PostgreSqlDialect {}, &POSTGRES_POLICY)
}

pub fn prepare_mysql_sql(sql: &str, max_rows: usize) -> Result<String, ConnectionError> {
    prepare(sql, max_rows, &MySqlDialect {}, &MYSQL_POLICY)
}

pub fn prepare_duckdb_sql(sql: &str, max_rows: usize) -> Result<String, ConnectionError> {
    prepare(sql, max_rows, &DuckDbDialect {}, &DUCKDB_POLICY)
}

pub fn prepare_snowflake_sql(sql: &str, max_rows: usize) -> Result<String, ConnectionError> {
    prepare(sql, max_rows, &SnowflakeDialect {}, &SNOWFLAKE_POLICY)
}

pub fn prepare_sqlite_sql(sql: &str, max_rows: usize) -> Result<String, ConnectionError> {
    prepare(sql, max_rows, &SQLiteDialect {}, &SQLITE_POLICY)
}

fn prepare(
    sql: &str,
    max_rows: usize,
    dialect: &dyn Dialect,
    policy: &BackendPolicy,
) -> Result<String, ConnectionError> {
    if max_rows == 0 {
        return Err(rejected());
    }
    let mut statements = Parser::parse_sql(dialect, sql).map_err(|_| rejected())?;
    let mut guard = Guard { policy };
    if statements.len() != 1 || statements.visit(&mut guard).is_break() || !allowed(&statements[0])
    {
        return Err(rejected());
    }
    if let Statement::Query(query) = &mut statements[0] {
        cap(query, max_rows);
    }
    Ok(statements.remove(0).to_string())
}

fn cap(query: &mut Query, max_rows: usize) {
    if literal(query.limit.as_ref()).is_some_and(|limit| limit <= max_rows) {
        return;
    }
    query.limit = Some(sqlparser::ast::Expr::Value(sqlparser::ast::Value::Number(
        max_rows.saturating_add(1).to_string(),
        false,
    )));
}

fn literal(limit: Option<&sqlparser::ast::Expr>) -> Option<usize> {
    match limit {
        Some(sqlparser::ast::Expr::Value(sqlparser::ast::Value::Number(value, _))) => {
            value.parse().ok()
        }
        _ => None,
    }
}

fn allowed(statement: &Statement) -> bool {
    match statement {
        Statement::Query(query) => query_allowed(query),
        Statement::Explain { statement, .. } => {
            matches!(statement.as_ref(), Statement::Query(query) if query_allowed(query))
        }
        Statement::ShowVariable { .. }
        | Statement::ShowVariables { .. }
        | Statement::ShowStatus { .. }
        | Statement::ShowCreate { .. }
        | Statement::ShowColumns { .. }
        | Statement::ShowDatabases { .. }
        | Statement::ShowSchemas { .. }
        | Statement::ShowTables { .. }
        | Statement::ShowViews { .. }
        | Statement::ShowFunctions { .. }
        | Statement::ShowCollation { .. } => true,
        _ => false,
    }
}

fn query_allowed(query: &Query) -> bool {
    query
        .with
        .as_ref()
        .is_none_or(|with| with.cte_tables.iter().all(|cte| query_allowed(&cte.query)))
        && set_allowed(&query.body)
}

fn set_allowed(set: &SetExpr) -> bool {
    match set {
        SetExpr::Select(select) => select.into.is_none(),
        SetExpr::Query(query) => query_allowed(query),
        SetExpr::SetOperation { left, right, .. } => set_allowed(left) && set_allowed(right),
        SetExpr::Values(_) => true,
        SetExpr::Insert(_) | SetExpr::Update(_) | SetExpr::Table(_) => false,
    }
}

fn rejected() -> ConnectionError {
    ConnectionError::query_failed("query rejected by read-only safety policy")
}

struct Guard<'a> {
    policy: &'a BackendPolicy,
}

impl Visitor for Guard<'_> {
    type Break = ();

    fn pre_visit_expr(&mut self, expr: &Expr) -> ControlFlow<Self::Break> {
        matches!(expr, Expr::Function(function) if denied(&function.name, self.policy))
            .then_some(())
            .map_or(ControlFlow::Continue(()), ControlFlow::Break)
    }

    fn pre_visit_relation(&mut self, relation: &ObjectName) -> ControlFlow<Self::Break> {
        denied(relation, self.policy)
            .then_some(())
            .map_or(ControlFlow::Continue(()), ControlFlow::Break)
    }
}

fn denied(name: &ObjectName, policy: &BackendPolicy) -> bool {
    let name = name.to_string().trim_matches('"').to_ascii_lowercase();
    COMMON_DENIED_FUNCTIONS.contains(&name.as_str())
        || policy.denied_functions.contains(&name.as_str())
        || policy
            .denied_prefixes
            .iter()
            .any(|prefix| name.starts_with(prefix))
}

#[cfg(test)]
mod tests {
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

    #[test]
    fn test_snowflake_denied_prefixes() {
        assert!(prepare_snowflake_sql("SELECT system$type('x')", 10).is_err());
        assert!(prepare_snowflake_sql("SELECT * FROM @stage", 10).is_err());

        assert!(prepare_postgres_sql("SELECT system$type('x')", 10).is_ok());
    }

    #[test]
    fn test_snowflake_denied_functions() {
        assert!(
            prepare_snowflake_sql("SELECT GET_PRESIGNED_URL(@stage, 'secret.csv')", 10).is_err()
        );
        assert!(
            prepare_snowflake_sql("SELECT BUILD_SCOPED_FILE_URL(@stage, 'secret.csv')", 10)
                .is_err()
        );
        assert!(prepare_snowflake_sql("SELECT * FROM DIRECTORY(@stage)", 10).is_err());

        assert!(prepare_duckdb_sql("SELECT * FROM read_csv('x')", 10).is_err());
        assert!(
            prepare_postgres_sql("SELECT GET_PRESIGNED_URL('stage', 'secret.csv')", 10).is_ok()
        );
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
}
