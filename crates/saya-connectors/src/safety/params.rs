//! Parameter-aware preparation for saved-investigation SQL.
//!
//! Portable saved SQL carries named `:name` placeholders. The entries here
//! prove the placeholders are exactly the names the run binds and return
//! the SQL rewritten to the dialect's native markers beside an ordered
//! value list — values are never substituted into the text. Statement
//! validation is the shared read-only pipeline of [`super::read_only`],
//! not a fork; the AST walks live in [`rewrite`].

use saya_types::{BoundParam, ConnectionError, ParamValue, SqlDialect, is_valid_param_name};

use super::read_only::{parse_guarded, parse_single_statement, parser_dialect};
use super::read_only_policy::{
    BIGQUERY_POLICY, BackendPolicy, DUCKDB_POLICY, MYSQL_POLICY, POSTGRES_POLICY, SNOWFLAKE_POLICY,
    SQLITE_POLICY,
};

mod rewrite;

use rewrite::{collect_names, rewrite_markers};

/// The bind-marker style a dialect natively accepts.
pub(super) enum Marker {
    Dollar,
    Question,
}

/// The read-only policy and bind-marker style a dialect supports. ClickHouse
/// and any dialect without a wired connector are refused before parsing.
fn support(dialect: SqlDialect) -> Option<(&'static BackendPolicy, Marker)> {
    match dialect {
        SqlDialect::Postgres => Some((&POSTGRES_POLICY, Marker::Dollar)),
        SqlDialect::Mysql => Some((&MYSQL_POLICY, Marker::Question)),
        SqlDialect::Sqlite => Some((&SQLITE_POLICY, Marker::Question)),
        SqlDialect::DuckDb => Some((&DUCKDB_POLICY, Marker::Question)),
        SqlDialect::Snowflake => Some((&SNOWFLAKE_POLICY, Marker::Question)),
        SqlDialect::BigQuery => Some((&BIGQUERY_POLICY, Marker::Question)),
        _ => None,
    }
}

fn parameters_unsupported(dialect: SqlDialect) -> ConnectionError {
    let name = match dialect {
        SqlDialect::ClickHouse => "ClickHouse".to_owned(),
        other => format!("the {other:?} dialect"),
    };
    ConnectionError::unsupported(format!("parameters are not supported for {name}"))
}

/// A prepared read-only query: the rewritten SQL text and the values in
/// marker order. Values live here only; nothing may persist them. The
/// Debug output redacts the values via [`ParamValue`]'s Debug.
#[derive(Debug)]
pub struct PreparedQuery {
    pub sql: String,
    pub values: Vec<ParamValue>,
}

/// The distinct `:name` placeholder names in a statement, in AST order of
/// first occurrence. Any other placeholder form (`$n`, `?`, `?1`, `@x`) —
/// or a `:name` outside the parameter-name grammar — is refused. This does
/// not run the read-only guard; pair it with the `prepare_*` gate.
pub fn sql_placeholders(sql: &str, dialect: SqlDialect) -> Result<Vec<String>, ConnectionError> {
    collect_names(&parse_single_statement(sql, parser_dialect(dialect))?)
}

/// Prepares read-only `sql` for a run that binds `params`: the guard, the
/// allow-list, and the row cap are exactly `prepare`'s; the placeholder
/// names must equal the bound names; each `:name` node becomes the
/// dialect's native marker and the values come back in marker order.
pub fn prepare_with_params(
    sql: &str,
    max_rows: usize,
    dialect: SqlDialect,
    params: &[BoundParam],
) -> Result<PreparedQuery, ConnectionError> {
    let (policy, style) = support(dialect).ok_or_else(|| parameters_unsupported(dialect))?;
    let bound = checked_bindings(params)?;
    let mut statement = parse_guarded(sql, max_rows, parser_dialect(dialect), policy)?;
    let names = collect_names(&statement)?;
    refuse_unbalanced(&names, &bound)?;
    let values = rewrite_markers(&mut statement, style, &bound);
    Ok(PreparedQuery {
        sql: statement.to_string(),
        values,
    })
}

/// The placeholder-name set and the bound-name set must be equal: every
/// missing and every extra name is reported by name, never a value.
fn refuse_unbalanced(
    names: &[String],
    bound: &[(&str, &ParamValue)],
) -> Result<(), ConnectionError> {
    let unbound: Vec<&str> = names
        .iter()
        .map(String::as_str)
        .filter(|name| !bound.iter().any(|(bound_name, _)| *bound_name == *name))
        .collect();
    if !unbound.is_empty() {
        return Err(ConnectionError::query_failed(format!(
            "no value was bound for the query's placeholder(s): {} — bind each with a parameter",
            unbound.join(", ")
        )));
    }
    let unused: Vec<&str> = bound
        .iter()
        .map(|(name, _)| *name)
        .filter(|name| !names.iter().any(|in_sql| in_sql == name))
        .collect();
    if !unused.is_empty() {
        return Err(ConnectionError::query_failed(format!(
            "a value was bound for parameter(s) the query does not use: {}",
            unused.join(", ")
        )));
    }
    Ok(())
}

/// Validates every bound name (grammar, uniqueness) and returns the name →
/// value pairs in binding order. Values are referenced, never copied.
fn checked_bindings(params: &[BoundParam]) -> Result<Vec<(&str, &ParamValue)>, ConnectionError> {
    let mut bound: Vec<(&str, &ParamValue)> = Vec::with_capacity(params.len());
    for param in params {
        if !is_valid_param_name(&param.name) {
            return Err(ConnectionError::query_failed(format!(
                "the bound parameter name {:?} is not a valid parameter name; names must match [a-z_][a-z0-9_]{{0,31}}",
                param.name
            )));
        }
        if bound.iter().any(|(name, _)| *name == param.name) {
            return Err(ConnectionError::query_failed(format!(
                "the parameter {} was bound more than once",
                param.name
            )));
        }
        bound.push((param.name.as_str(), &param.value));
    }
    Ok(bound)
}
