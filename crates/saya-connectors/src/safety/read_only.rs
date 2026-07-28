use saya_types::ConnectionError;
use std::ops::ControlFlow;

use sqlparser::{
    ast::{Expr, ObjectName, Query, SetExpr, Statement, Visit, Visitor},
    dialect::{Dialect, DuckDbDialect, MySqlDialect, PostgreSqlDialect, SnowflakeDialect},
    parser::Parser,
};

pub fn prepare_postgres_sql(sql: &str, max_rows: usize) -> Result<String, ConnectionError> {
    prepare(sql, max_rows, &PostgreSqlDialect {})
}

pub fn prepare_mysql_sql(sql: &str, max_rows: usize) -> Result<String, ConnectionError> {
    prepare(sql, max_rows, &MySqlDialect {})
}

pub fn prepare_duckdb_sql(sql: &str, max_rows: usize) -> Result<String, ConnectionError> {
    prepare(sql, max_rows, &DuckDbDialect {})
}

pub fn prepare_snowflake_sql(sql: &str, max_rows: usize) -> Result<String, ConnectionError> {
    prepare(sql, max_rows, &SnowflakeDialect {})
}

fn prepare(sql: &str, max_rows: usize, dialect: &dyn Dialect) -> Result<String, ConnectionError> {
    if max_rows == 0 {
        return Err(rejected());
    }
    let mut statements = Parser::parse_sql(dialect, sql).map_err(|_| rejected())?;
    let mut guard = Guard;
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
    ConnectionError::QueryFailed("query rejected by read-only safety policy".into())
}

struct Guard;

impl Visitor for Guard {
    type Break = ();

    fn pre_visit_expr(&mut self, expr: &Expr) -> ControlFlow<Self::Break> {
        matches!(expr, Expr::Function(function) if denied(&function.name))
            .then_some(())
            .map_or(ControlFlow::Continue(()), ControlFlow::Break)
    }

    fn pre_visit_relation(&mut self, relation: &ObjectName) -> ControlFlow<Self::Break> {
        denied(relation)
            .then_some(())
            .map_or(ControlFlow::Continue(()), ControlFlow::Break)
    }
}

fn denied(name: &ObjectName) -> bool {
    let name = name.to_string().trim_matches('"').to_ascii_lowercase();
    [
        "nextval",
        "setval",
        "read_csv",
        "read_csv_auto",
        "read_json",
        "read_json_auto",
        "read_parquet",
        "read_text",
        "sqlite_scan",
        "glob",
        "get_presigned_url",
        "build_scoped_file_url",
        "directory",
        "metadata",
    ]
    .contains(&name.as_str())
        || name.starts_with('@')
        || name.starts_with("system$")
}
