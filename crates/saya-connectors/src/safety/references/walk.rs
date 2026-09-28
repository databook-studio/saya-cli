//! The AST visitor that fills [`Extractor`].
//!
//! Split out of `references.rs` only for size; the rules it encodes belong
//! with the rest of the extraction contract. See the spec linked from the
//! parent module.

use std::collections::{HashMap, HashSet};
use std::ops::ControlFlow;

use sqlparser::ast::{Expr, ObjectName, Query, TableFactor, Visitor};

use super::{MAX_COLUMNS, MAX_OBJECTS};

/// One enclosing `WITH` whose aliases become visible as its CTE bodies
/// complete: all of them up front for `WITH RECURSIVE`, one at a time
/// otherwise. Scopes nest, so they are kept on a stack.
struct CteScope {
    /// Aliases this scope has made visible so far; removed again when the
    /// scope is left.
    activated: Vec<String>,
    /// Aliases of a non-recursive WITH whose bodies have not completed yet,
    /// in declaration order; empty for a recursive WITH.
    pending: Vec<String>,
    /// Queries currently open inside this scope's CTE bodies: 1 is the
    /// expected CTE body itself, more are nested queries within it.
    body_nesting: usize,
}

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
    /// CTE alias names visible at the current point of the walk, keyed by
    /// name with the number of enclosing scopes that define it — an alias
    /// may be defined at two nested scopes. A name is a CTE reference only
    /// while one of its defining scopes has activated it; CTEs are never
    /// objects.
    cte_scopes: HashMap<String, usize>,
    /// The enclosing `WITH` scopes, innermost last; see [`CteScope`].
    cte_frames: Vec<CteScope>,
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

    /// Enter a query into the enclosing CTE scope's body tracking, then open
    /// the query's own CTE scope. When the enclosing scope still expects a
    /// CTE body, this query is it: `with` is `Query`'s first visited field
    /// and a `WITH`'s only queries are its CTE bodies, so no other query can
    /// fire between the scope opening (or the previous body closing) and this
    /// one.
    fn pre_visit_query(&mut self, query: &Query) -> ControlFlow<Self::Break> {
        if let Some(scope) = self.cte_frames.last_mut() {
            if scope.body_nesting > 0 {
                scope.body_nesting += 1;
            } else if !scope.pending.is_empty() {
                scope.body_nesting = 1;
            }
        }
        if let Some(with) = &query.with {
            let names: Vec<String> = with
                .cte_tables
                .iter()
                .map(|cte| cte.alias.name.value.clone())
                .collect();
            if with.recursive {
                // Every alias is visible inside every CTE body at once: a
                // self-reference inside its own definition is the CTE.
                for name in &names {
                    *self.cte_scopes.entry(name.clone()).or_insert(0) += 1;
                }
                self.cte_frames.push(CteScope {
                    activated: names,
                    pending: Vec::new(),
                    body_nesting: 0,
                });
            } else {
                // No alias is visible while its own body is walked: it
                // activates when that body completes, so earlier bodies see
                // the base table and later bodies and the main body see the
                // CTE.
                self.cte_frames.push(CteScope {
                    activated: Vec::new(),
                    pending: names,
                    body_nesting: 0,
                });
            }
        }
        ControlFlow::Continue(())
    }

    /// Leave a query: close its own CTE scope, then record that a CTE body
    /// of the enclosing scope completed, activating the alias it defines.
    /// `Query::visit` pairs every `pre_visit_query` with this hook on the
    /// same node (only a `Break` in between would skip it, and
    /// `sql_references` flags that as `partial`), so scope is known for every
    /// construct this visitor models; constructs it does not model already
    /// set `partial` and are never excluded on scope grounds.
    fn post_visit_query(&mut self, query: &Query) -> ControlFlow<Self::Break> {
        if query.with.is_some()
            && let Some(scope) = self.cte_frames.pop()
        {
            for name in &scope.activated {
                if let Some(count) = self.cte_scopes.get_mut(name) {
                    *count -= 1;
                    if *count == 0 {
                        self.cte_scopes.remove(name);
                    }
                }
            }
            if !scope.pending.is_empty() {
                // A CTE body never completed, so alias visibility is no
                // longer trustworthy: fail closed rather than exclude.
                self.partial = true;
            }
        }
        if let Some(scope) = self.cte_frames.last_mut()
            && scope.body_nesting > 0
        {
            scope.body_nesting -= 1;
            if scope.body_nesting == 0 && !scope.pending.is_empty() {
                let name = scope.pending.remove(0);
                scope.activated.push(name.clone());
                *self.cte_scopes.entry(name).or_insert(0) += 1;
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
                if is_cte_reference(name, &self.cte_scopes) {
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
/// that matches a CTE alias visible at the reference's scope — CTEs are never
/// schema-qualified.
fn is_cte_reference(name: &ObjectName, cte_scopes: &HashMap<String, usize>) -> bool {
    name.0.len() == 1 && cte_scopes.contains_key(&name.0[0].value)
}

/// The parts of a relation name, exactly as written (case preserved).
fn parts_of(name: &ObjectName) -> Vec<String> {
    name.0.iter().map(|ident| ident.value.clone()).collect()
}
