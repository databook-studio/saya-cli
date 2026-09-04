//! Join fan-out reconciliation probe builder.
//!
//! A join on a non-unique key multiplies the rows on one side, inflating every
//! `SUM`/`AVG`/`COUNT` over the base table. The probe emits two `COUNT(*)`
//! statements — over the full join and over the base table alone under the same
//! filter — whose disagreement proves fan-out. This only builds SQL; it executes
//! nothing and carries no result-set values, only structure from the query.

use saya_types::SqlDialect;
use sqlparser::ast::{
    DuplicateTreatment, Expr, Function, FunctionArgumentList, FunctionArguments, Ident, Query,
    SelectItem, SetExpr, Statement, TableFactor, Visit, Visitor,
};
use sqlparser::parser::Parser;
use std::ops::ControlFlow;

use crate::safety::parser_dialect;

/// The two counts whose disagreement proves a join fanned out.
pub struct FanoutProbe {
    /// Rows actually feeding the aggregate, i.e. the full `FROM` with its joins.
    pub joined_rows: String,
    /// Rows in the base table alone under the same filter.
    pub base_rows: String,
}

/// Build a fan-out probe for `sql`, or `None` when a sound probe cannot be
/// built. Returning `None` is always safe; a probe not equivalent to the
/// original query is worse than none, so every uncertain shape is refused.
pub fn fanout_probe(sql: &str, dialect: SqlDialect) -> Option<FanoutProbe> {
    let mut statements = Parser::parse_sql(parser_dialect(dialect), sql).ok()?;
    if statements.len() != 1 {
        return None;
    }
    let query = match statements.remove(0) {
        Statement::Query(q) => q,
        _ => return None,
    };
    if query.with.is_some() {
        return None;
    }
    let select = match *query.body {
        SetExpr::Select(s) => s,
        _ => return None,
    };
    if select.into.is_some() || select.from.len() != 1 {
        return None;
    }
    let from = &select.from[0];
    if from.joins.is_empty()
        || !is_plain_table(&from.relation)
        || !from.joins.iter().all(|j| is_plain_table(&j.relation))
    {
        return None;
    }
    if !select.projection.iter().any(has_distortable_aggregate) {
        return None;
    }
    if let Some(selection) = &select.selection
        && !where_touches_only_base(selection, BaseRef::of(&from.relation))
    {
        return None;
    }
    let where_clause = select
        .selection
        .as_ref()
        .map_or(String::new(), |w| format!(" WHERE {w}"));
    Some(FanoutProbe {
        joined_rows: format!("SELECT COUNT(*) AS n FROM {from}{where_clause}"),
        base_rows: format!("SELECT COUNT(*) AS n FROM {}{where_clause}", from.relation),
    })
}

/// A plain base table. Subqueries, derived tables, and table functions in
/// `FROM` change the base or join shape, so they are refused upstream.
fn is_plain_table(factor: &TableFactor) -> bool {
    matches!(factor, TableFactor::Table { args: None, .. })
}

/// A projection item holds a fan-out-susceptible aggregate: `SUM`/`AVG`/`COUNT`
/// without `DISTINCT` (`MIN`/`MAX` and `DISTINCT` collapse duplicates). Aggregates
/// inside a scalar subquery belong to that subquery's scope, not the outer join.
fn has_distortable_aggregate(item: &SelectItem) -> bool {
    let expr = match item {
        SelectItem::UnnamedExpr(e) | SelectItem::ExprWithAlias { expr: e, .. } => e,
        _ => return false,
    };
    let mut scan = ScopeScan::default();
    let _ = expr.visit(&mut scan);
    scan.found
}

fn is_distortable(f: &Function) -> bool {
    // Only an unqualified `SUM`/`AVG`/`COUNT` without `DISTINCT` is a built-in
    // aggregate that fan-out distorts; a schema-qualified name is a custom
    // function, and `DISTINCT` collapses the duplicated rows.
    let name = match f.name.0.as_slice() {
        [single] => single,
        _ => return false,
    };
    matches!(
        name.value.to_ascii_lowercase().as_str(),
        "sum" | "avg" | "count"
    ) && !matches!(
        &f.args,
        FunctionArguments::List(FunctionArgumentList {
            duplicate_treatment: Some(DuplicateTreatment::Distinct),
            ..
        })
    )
}

/// The base (first) table's identifying names, for `WHERE` qualifier checks.
struct BaseRef {
    alias: Option<String>,
    name: Vec<String>,
}

impl BaseRef {
    fn of(factor: &TableFactor) -> Self {
        match factor {
            TableFactor::Table { name, alias, .. } => Self {
                alias: alias.as_ref().map(|a| a.name.value.clone()),
                name: name.0.iter().map(|i| i.value.clone()).collect(),
            },
            _ => unreachable!("base is a plain table; checked before construction"),
        }
    }

    /// `qualifier` is how a base-table column is written: the alias, or the
    /// table name (full or last part) when unaliased. Anything else is joined.
    fn accepts_qualifier(&self, qualifier: &[Ident]) -> bool {
        if let Some(alias) = &self.alias {
            return qualifier.len() == 1 && qualifier[0].value.eq_ignore_ascii_case(alias);
        }
        let full = qualifier.len() == self.name.len()
            && qualifier
                .iter()
                .zip(&self.name)
                .all(|(q, n)| q.value.eq_ignore_ascii_case(n));
        let last = self.name.last().map(String::as_str).unwrap_or("");
        full || (qualifier.len() == 1 && qualifier[0].value.eq_ignore_ascii_case(last))
    }
}

/// Depth-tracking scan: only the outer query's own expressions (depth 0) are
/// inspected. Anything inside a subquery belongs to that subquery's scope, so
/// its aggregates are not counted and its column qualifiers are not policed.
#[derive(Default)]
struct ScopeScan {
    depth: usize,
    /// Set when a fan-out-susceptible aggregate is found (aggregate mode).
    found: bool,
    /// Set when a non-base qualifier is seen in the WHERE (where mode).
    bad: bool,
    /// `Some` in where mode (the base to check against); `None` in aggregate mode.
    base: Option<BaseRef>,
}

impl Visitor for ScopeScan {
    type Break = ();

    fn pre_visit_query(&mut self, _q: &Query) -> ControlFlow<()> {
        self.depth += 1;
        ControlFlow::Continue(())
    }

    fn post_visit_query(&mut self, _q: &Query) -> ControlFlow<()> {
        self.depth -= 1;
        ControlFlow::Continue(())
    }

    fn pre_visit_expr(&mut self, expr: &Expr) -> ControlFlow<()> {
        if self.depth != 0 {
            return ControlFlow::Continue(());
        }
        match &self.base {
            Some(base) => {
                if let Expr::CompoundIdentifier(parts) = expr
                    && parts.len() >= 2
                    && !base.accepts_qualifier(&parts[..parts.len() - 1])
                {
                    self.bad = true;
                    return ControlFlow::Break(());
                }
            }
            None => {
                if let Expr::Function(f) = expr
                    && is_distortable(f)
                {
                    self.found = true;
                    return ControlFlow::Break(());
                }
            }
        }
        ControlFlow::Continue(())
    }
}

/// A `WHERE` carries onto the base table only when its top-level columns are
/// unqualified or qualified with the base table's name/alias.
fn where_touches_only_base(selection: &Expr, base: BaseRef) -> bool {
    let mut scan = ScopeScan {
        base: Some(base),
        ..Default::default()
    };
    let _ = selection.visit(&mut scan);
    !scan.bad
}
