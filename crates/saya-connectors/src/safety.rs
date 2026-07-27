use sqlparser::{
    ast::{Query, SetExpr, Statement},
    dialect::PostgreSqlDialect,
    parser::Parser,
};

use saya_types::ConnectionError;

/// Parses a single PostgreSQL read-only statement and caps SELECT results.
pub fn prepare_postgres_sql(sql: &str, max_rows: usize) -> Result<String, ConnectionError> {
    if max_rows == 0 {
        return Err(rejected());
    }
    let dialect = PostgreSqlDialect {};
    let mut statements = Parser::parse_sql(&dialect, sql).map_err(|_| rejected())?;
    if statements.len() != 1 || !allowed(&statements[0]) {
        return Err(rejected());
    }
    if let Statement::Query(query) = &mut statements[0] {
        let observed = max_rows.saturating_add(1);
        query.limit = Some(sqlparser::ast::Expr::Value(sqlparser::ast::Value::Number(
            observed.to_string(),
            false,
        )));
    }
    Ok(statements.remove(0).to_string())
}

fn allowed(statement: &Statement) -> bool {
    match statement {
        Statement::Query(query) => query_allowed(query),
        Statement::Explain { statement, .. } => match statement.as_ref() {
            Statement::Query(query) => query_allowed(query),
            _ => false,
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
        SetExpr::Values(_) | SetExpr::Insert(_) | SetExpr::Update(_) | SetExpr::Table(_) => false,
    }
}

fn rejected() -> ConnectionError {
    ConnectionError::QueryFailed("query rejected by read-only PostgreSQL safety policy".into())
}

#[cfg(test)]
mod tests {
    use super::prepare_postgres_sql;

    #[test]
    fn bounds_and_observes_one_extra_row() {
        assert!(
            prepare_postgres_sql("select * from events", 5)
                .unwrap()
                .contains("LIMIT 6")
        );
        assert!(
            prepare_postgres_sql("select 1 limit 2", 5)
                .unwrap()
                .contains("LIMIT 6")
        );
    }

    #[test]
    fn rejects_select_into_and_data_modifying_ctes() {
        assert!(prepare_postgres_sql("SELECT * INTO archive FROM events", 5).is_err());
        assert!(
            prepare_postgres_sql("WITH x AS (INSERT INTO t VALUES (1)) SELECT * FROM x", 5)
                .is_err()
        );
    }
}
