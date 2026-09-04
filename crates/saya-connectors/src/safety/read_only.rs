use saya_types::{ConnectionError, SqlDialect};
use std::ops::ControlFlow;

use sqlparser::{
    ast::{Expr, Query, SetExpr, Statement, Visit, Visitor},
    dialect::{
        ClickHouseDialect, Dialect, DuckDbDialect, MySqlDialect, PostgreSqlDialect, SQLiteDialect,
        SnowflakeDialect,
    },
    parser::Parser,
};

use super::read_only_policy::{
    BackendPolicy, CLICKHOUSE_POLICY, DUCKDB_POLICY, MYSQL_POLICY, POSTGRES_POLICY,
    SNOWFLAKE_POLICY, SQLITE_POLICY, denied_function, denied_relation,
};
use super::reject::{Rejection, kind, rejected};

/// The single place that maps a [`SqlDialect`] to the `sqlparser` dialect the
/// safety layer parses with. Shared by the read-only `prepare_*` functions and
/// by object/column extraction so the two never drift apart.
pub(super) fn parser_dialect(dialect: SqlDialect) -> &'static dyn Dialect {
    match dialect {
        SqlDialect::Postgres => &PostgreSqlDialect {},
        SqlDialect::Mysql => &MySqlDialect {},
        SqlDialect::DuckDb => &DuckDbDialect,
        SqlDialect::Snowflake => &SnowflakeDialect,
        SqlDialect::Sqlite => &SQLiteDialect {},
        SqlDialect::ClickHouse => &ClickHouseDialect {},
        // `SqlDialect` is `#[non_exhaustive]`. A dialect added later must be
        // wired in explicitly; until then parse as Postgres (the broadest of the
        // five) so the safety layer still rejects or accepts based on syntax.
        _ => &PostgreSqlDialect {},
    }
}

pub fn prepare_postgres_sql(sql: &str, max_rows: usize) -> Result<String, ConnectionError> {
    prepare(
        sql,
        max_rows,
        parser_dialect(SqlDialect::Postgres),
        &POSTGRES_POLICY,
    )
}

pub fn prepare_mysql_sql(sql: &str, max_rows: usize) -> Result<String, ConnectionError> {
    prepare(
        sql,
        max_rows,
        parser_dialect(SqlDialect::Mysql),
        &MYSQL_POLICY,
    )
}

pub fn prepare_duckdb_sql(sql: &str, max_rows: usize) -> Result<String, ConnectionError> {
    prepare(
        sql,
        max_rows,
        parser_dialect(SqlDialect::DuckDb),
        &DUCKDB_POLICY,
    )
}

pub fn prepare_snowflake_sql(sql: &str, max_rows: usize) -> Result<String, ConnectionError> {
    prepare(
        sql,
        max_rows,
        parser_dialect(SqlDialect::Snowflake),
        &SNOWFLAKE_POLICY,
    )
}

pub fn prepare_sqlite_sql(sql: &str, max_rows: usize) -> Result<String, ConnectionError> {
    prepare(
        sql,
        max_rows,
        parser_dialect(SqlDialect::Sqlite),
        &SQLITE_POLICY,
    )
}

pub fn prepare_clickhouse_sql(sql: &str, max_rows: usize) -> Result<String, ConnectionError> {
    prepare(
        sql,
        max_rows,
        parser_dialect(SqlDialect::ClickHouse),
        &CLICKHOUSE_POLICY,
    )
}

fn prepare(
    sql: &str,
    max_rows: usize,
    dialect: &dyn Dialect,
    policy: &BackendPolicy,
) -> Result<String, ConnectionError> {
    if max_rows == 0 {
        return Err(rejected(Rejection::RowCap));
    }
    let mut statements = Parser::parse_sql(dialect, sql).map_err(|_| rejected(Rejection::Parse))?;
    if statements.len() != 1 {
        return Err(rejected(Rejection::MultipleStatements));
    }
    let mut guard = Guard {
        policy,
        denied: None,
    };
    if statements.visit(&mut guard).is_break() {
        return Err(rejected(
            guard
                .denied
                .unwrap_or(Rejection::Denied("this construct".to_string())),
        ));
    }
    allowed(&statements[0]).map_err(rejected)?;
    if let Some(query) = statement_query(&mut statements[0]) {
        if policy.deny_format_clause && query.format_clause.is_some() {
            return Err(rejected(Rejection::FormatClause));
        }
        cap(query, max_rows);
    }
    Ok(statements.remove(0).to_string())
}

/// The query a statement executes, for row-cap injection. `allowed()` has
/// already narrowed this to `Query` and `Explain(Query)`; capping the inner
/// query of `EXPLAIN ANALYZE` matters because that form *executes* the plan.
fn statement_query(statement: &mut Statement) -> Option<&mut Query> {
    match statement {
        Statement::Query(query) => Some(query),
        Statement::Explain { statement, .. } => match statement.as_mut() {
            Statement::Query(query) => Some(query),
            _ => None,
        },
        _ => None,
    }
}

fn cap(query: &mut Query, max_rows: usize) {
    let limit = literal(query.limit.as_ref());
    let fetch = query
        .fetch
        .as_ref()
        .and_then(|fetch| literal(fetch.quantity.as_ref()));
    // An explicit bound at or under the cap is left exactly as written,
    // including `FETCH FIRST … ROWS ONLY` (Postgres rejects LIMIT+FETCH).
    if limit.or(fetch).is_some_and(|bound| bound <= max_rows) {
        return;
    }
    query.limit = Some(sqlparser::ast::Expr::Value(sqlparser::ast::Value::Number(
        max_rows.saturating_add(1).to_string(),
        false,
    )));
    // The injected LIMIT replaces any looser FETCH clause rather than
    // combining with it.
    query.fetch = None;
}

fn literal(limit: Option<&sqlparser::ast::Expr>) -> Option<usize> {
    match limit {
        Some(sqlparser::ast::Expr::Value(sqlparser::ast::Value::Number(value, _))) => {
            value.parse().ok()
        }
        _ => None,
    }
}

fn allowed(statement: &Statement) -> Result<(), Rejection> {
    match statement {
        Statement::Query(query) => query_allowed(query),
        Statement::Explain { statement, .. } => match statement.as_ref() {
            Statement::Query(query) => query_allowed(query),
            other => Err(Rejection::WriteStatement(kind(other))),
        },
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
        | Statement::ShowCollation { .. } => Ok(()),
        _ => Err(Rejection::WriteStatement(kind(statement))),
    }
}

fn query_allowed(query: &Query) -> Result<(), Rejection> {
    // Row-locking clauses (`FOR UPDATE` / `FOR SHARE`) are rejected by the
    // `Guard` visitor's `pre_visit_query`, which fires for *every* `Query` in
    // the tree — top level, CTEs, set operands, derived tables, and scalar
    // subqueries alike — so this structural walk does not re-check them.
    if let Some(with) = &query.with
        && !with
            .cte_tables
            .iter()
            .all(|cte| query_allowed(&cte.query).is_ok())
    {
        // Propagate the *inner* reason: a CTE wrapping an INSERT should say
        // so, not blame the wrapper.
        return with
            .cte_tables
            .iter()
            .find_map(|cte| query_allowed(&cte.query).err())
            .map_or_else(|| Err(Rejection::WriteStatement("CTE")), Err);
    }
    set_allowed(&query.body)
}

fn set_allowed(set: &SetExpr) -> Result<(), Rejection> {
    match set {
        SetExpr::Select(select) => select
            .into
            .is_none()
            .then_some(())
            .ok_or(Rejection::WriteStatement("SELECT INTO")),
        SetExpr::Query(query) => query_allowed(query),
        SetExpr::SetOperation { left, right, .. } => {
            set_allowed(left)?;
            set_allowed(right)
        }
        SetExpr::Values(_) => Ok(()),
        SetExpr::Insert(_) => Err(Rejection::WriteStatement("INSERT")),
        SetExpr::Update(_) => Err(Rejection::WriteStatement("UPDATE")),
        SetExpr::Table(_) => Err(Rejection::WriteStatement("this statement")),
    }
}

struct Guard<'a> {
    policy: &'a BackendPolicy,
    /// The rejection to surface, set on the first denying node we reach.
    denied: Option<Rejection>,
}

impl Visitor for Guard<'_> {
    type Break = ();

    fn pre_visit_query(&mut self, query: &Query) -> ControlFlow<Self::Break> {
        // Row-locking clauses take locks and are not reads. `pre_visit_query`
        // fires for every `Query` node in the tree, so this reaches locks in
        // CTEs, set operands, derived tables, and scalar subqueries — not just
        // the top level.
        if !query.locks.is_empty() {
            self.denied = Some(Rejection::LockingClause);
            return ControlFlow::Break(());
        }
        ControlFlow::Continue(())
    }

    fn pre_visit_expr(&mut self, expr: &Expr) -> ControlFlow<Self::Break> {
        if let Expr::Function(function) = expr
            && denied_function(&function.name, self.policy)
        {
            self.denied = Some(Rejection::Denied(function.name.to_string()));
            return ControlFlow::Break(());
        }
        ControlFlow::Continue(())
    }

    fn pre_visit_table_factor(
        &mut self,
        factor: &sqlparser::ast::TableFactor,
    ) -> ControlFlow<Self::Break> {
        use sqlparser::ast::TableFactor;
        match factor {
            // A table *function* in `FROM` (`SELECT * FROM pg_read_file(...)`)
            // carries arguments. Per-part matching makes schema qualification
            // (`pg_catalog.pg_read_file`) no help.
            TableFactor::Table {
                name,
                args: Some(_),
                ..
            } => {
                if denied_function(name, self.policy) {
                    self.denied = Some(Rejection::Denied(name.to_string()));
                    return ControlFlow::Break(());
                }
                ControlFlow::Continue(())
            }
            // `LATERAL fn(...)` and similar function-valued table factors carry
            // their own `name`; apply the same per-part function check.
            TableFactor::Function { name, .. } => {
                if denied_function(name, self.policy) {
                    self.denied = Some(Rejection::Denied(name.to_string()));
                    return ControlFlow::Break(());
                }
                ControlFlow::Continue(())
            }
            // A plain table reference (`args: None`) keeps the stricter
            // whole-name check so a table literally named like a denied
            // function (e.g. `nextval`) stays blocked — fail-closed by design.
            TableFactor::Table {
                name, args: None, ..
            } => {
                if denied_relation(name, self.policy) {
                    self.denied = Some(Rejection::Denied(name.to_string()));
                    return ControlFlow::Break(());
                }
                ControlFlow::Continue(())
            }
            // Derived tables, UNNEST, JSON_TABLE, etc. carry no function name
            // of their own; the visitor recurses into their subqueries and
            // expressions, which the checks above cover.
            _ => ControlFlow::Continue(()),
        }
    }
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
