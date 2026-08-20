//! The AST visitor that fills [`Extractor`].
//!
//! Split out of `references.rs` only for size; the rules it encodes belong
//! with the rest of the extraction contract. See the spec linked from the
//! parent module.

use std::collections::HashSet;
use std::ops::ControlFlow;

use sqlparser::ast::{Expr, ObjectName, Query, TableFactor, Visitor};

use super::{MAX_COLUMNS, MAX_OBJECTS};

/// Collects object and column names while the visitor walks a single `Query`.
///
/// Field names (`objects`, `columns`, `partial`) are read by `sql_references`,
/// so they are part of the parent module's contract. The dedup and CTE state
/// kept here is an implementation detail.
#[derive(Default)]
pub struct Extractor {
    /// Table-ish names as written, each split into its parts (1, 2 or 3).
    pub objects: Vec<Vec<String>>,
    /// Column identifiers as written; unqualified names appear bare.
    pub columns: Vec<String>,
    /// `true` when an unmodelled construct was seen; lists may be incomplete.
    pub partial: bool,
    /// CTE alias names collected from every `WITH` — never objects.
    cte_names: HashSet<String>,
    seen_objects: HashSet<Vec<String>>,
    seen_columns: HashSet<String>,
}

impl Extractor {
    /// Record a base table's parts, deduplicated in first-seen order. The bound
    /// truncates and flags `partial` rather than growing without limit.
    fn record_object(&mut self, parts: Vec<String>) {
        if self.seen_objects.contains(&parts) {
            return;
        }
        self.seen_objects.insert(parts.clone());
        if self.objects.len() < MAX_OBJECTS {
            self.objects.push(parts);
        } else {
            self.partial = true;
        }
    }

    /// Record a column's (last) identifier part, deduplicated in first-seen
    /// order. The bound truncates and flags `partial`.
    fn record_column(&mut self, name: String) {
        if self.seen_columns.contains(&name) {
            return;
        }
        self.seen_columns.insert(name.clone());
        if self.columns.len() < MAX_COLUMNS {
            self.columns.push(name);
        } else {
            self.partial = true;
        }
    }
}

impl Visitor for Extractor {
    type Break = ();

    /// Collect CTE aliases before any relation in the same query is visited:
    /// `WITH` precedes `body`, and `pre_visit_query` fires before children.
    fn pre_visit_query(&mut self, query: &Query) -> ControlFlow<Self::Break> {
        if let Some(with) = &query.with {
            for cte in &with.cte_tables {
                self.cte_names.insert(cte.alias.name.value.clone());
            }
        }
        ControlFlow::Continue(())
    }

    /// Objects are recorded here, not in `pre_visit_relation`: only a table
    /// factor carries enough context to tell a base table from a table-valued
    /// function. Within a `Query` the only relation that fires is this
    /// factor's own `name`, so handling it once here is exact.
    fn pre_visit_table_factor(&mut self, factor: &TableFactor) -> ControlFlow<Self::Break> {
        match factor {
            TableFactor::Table { name, args, .. } => {
                // A table-valued function (`generate_series(...)`) parses as a
                // `Table` with arguments: its name is a function, not a table.
                if args.is_some() {
                    self.partial = true;
                    return ControlFlow::Continue(());
                }
                if is_cte_reference(name, &self.cte_names) {
                    return ControlFlow::Continue(());
                }
                self.record_object(parts_of(name));
            }
            TableFactor::Derived { .. } | TableFactor::NestedJoin { .. } => {
                // A subquery or parenthesised join contributes its own factors
                // as they are visited; nothing extra to model here.
            }
            // Function, TableFunction, UNNEST, JSON_TABLE, OPENJSON, and the
            // pivot-family wrappers name a function or a transform, not a base
            // table: stay out of `objects` and mark the result partial.
            _ => self.partial = true,
        }
        ControlFlow::Continue(())
    }

    /// Columns are the leaf identifiers of expressions. A qualified name keeps
    /// only its last part, so `o.id` -> `id`; a string literal is `Expr::Value`
    /// and never reaches here.
    fn pre_visit_expr(&mut self, expr: &Expr) -> ControlFlow<Self::Break> {
        match expr {
            Expr::Identifier(ident) => self.record_column(ident.value.clone()),
            Expr::CompoundIdentifier(parts) => {
                if let Some(last) = parts.last() {
                    self.record_column(last.value.clone());
                }
            }
            _ => {}
        }
        ControlFlow::Continue(())
    }
}

/// A relation is a CTE reference only when it is a single, unqualified name
/// that matches a known CTE alias — CTEs are never schema-qualified.
fn is_cte_reference(name: &ObjectName, cte_names: &HashSet<String>) -> bool {
    name.0.len() == 1 && cte_names.contains(&name.0[0].value)
}

/// The parts of a relation name, exactly as written (case preserved).
fn parts_of(name: &ObjectName) -> Vec<String> {
    name.0.iter().map(|ident| ident.value.clone()).collect()
}
