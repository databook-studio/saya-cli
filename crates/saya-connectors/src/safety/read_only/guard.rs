use saya_types::ConnectionError;
use std::ops::ControlFlow;

use sqlparser::ast::{Expr, Query, SetExpr, Statement, Visit, Visitor};

use super::super::read_only_policy::{BackendPolicy, denied_function, denied_relation};
use super::super::reject::{Rejection, kind, rejected};

pub(super) fn guard_statement(
    statement: &mut Statement,
    policy: &BackendPolicy,
) -> Result<(), ConnectionError> {
    let mut guard = Guard {
        policy,
        denied: None,
    };
    if statement.visit(&mut guard).is_break() {
        return Err(rejected(
            guard
                .denied
                .unwrap_or(Rejection::Denied("this construct".to_string())),
        ));
    }
    allowed(statement).map_err(rejected)
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
