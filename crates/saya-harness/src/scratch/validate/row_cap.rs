//! The row cap: a read the model sees is bounded at the statement level,
//! so the collection layer can report `truncated` instead of silently
//! cutting.

use sqlparser::ast::{Query, Statement};

/// The query a statement executes, for row-cap injection. `allowed()` has
/// already narrowed this to `Query` and `Explain(Query)`; capping the inner
/// query of `EXPLAIN ANALYZE` matters because that form *executes* the plan.
pub(super) fn top_query(statement: &mut Statement) -> Option<&mut Query> {
    match statement {
        Statement::Query(query) => Some(query),
        Statement::Explain { statement, .. } => match statement.as_mut() {
            Statement::Query(query) => Some(query),
            _ => None,
        },
        _ => None,
    }
}

/// Injects the row cap into a read the way the connector's gate does: an
/// explicit bound at or under the cap is left as written; anything looser is
/// replaced by `LIMIT cap+1` (any FETCH dropped) so the collection layer can
/// report `truncated` instead of silently cutting.
pub(super) fn cap(query: &mut Query, max_rows: usize) {
    let limit = literal(query.limit.as_ref());
    let fetch = query
        .fetch
        .as_ref()
        .and_then(|fetch| literal(fetch.quantity.as_ref()));
    if limit.or(fetch).is_some_and(|bound| bound <= max_rows) {
        return;
    }
    query.limit = Some(sqlparser::ast::Expr::Value(sqlparser::ast::Value::Number(
        max_rows.saturating_add(1).to_string(),
        false,
    )));
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
