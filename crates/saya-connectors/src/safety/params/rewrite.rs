//! The AST walks for parameter-aware preparation: extracting `:name`
//! placeholder names and rewriting each placeholder node to the dialect's
//! native bind marker. Markers are written into `Value::Placeholder`
//! nodes, which Display then prints verbatim — no string substitution.

use std::collections::{HashMap, HashSet};
use std::ops::ControlFlow;

use saya_types::{ConnectionError, ParamValue, is_valid_param_name};
use sqlparser::ast::{Expr, Statement, Value, Visit, VisitMut, Visitor, VisitorMut};

use super::Marker;

/// `":region"` → `"region"`; any other placeholder form has no portable
/// name and keeps its raw text for the refusal message.
fn name_of(raw: &str) -> Option<String> {
    raw.strip_prefix(':')
        .filter(|name| is_valid_param_name(name))
        .map(str::to_owned)
}

/// The distinct `:name` placeholder names in `statement`, in AST order of
/// first occurrence; the first placeholder that is not one is refused.
pub(super) fn collect_names(statement: &Statement) -> Result<Vec<String>, ConnectionError> {
    let mut collector = NameCollector {
        names: Vec::new(),
        seen: HashSet::new(),
        refused: None,
    };
    let _ = statement.visit(&mut collector);
    if let Some(raw) = collector.refused {
        return Err(ConnectionError::query_failed(format!(
            "the placeholder {raw} is not a :name placeholder; saved SQL must use :name placeholders for parameters"
        )));
    }
    Ok(collector.names)
}

/// Collects placeholder names, deduplicating on first occurrence.
struct NameCollector {
    names: Vec<String>,
    seen: HashSet<String>,
    refused: Option<String>,
}

impl Visitor for NameCollector {
    type Break = ();

    fn post_visit_expr(&mut self, expr: &Expr) -> ControlFlow<()> {
        let Expr::Value(Value::Placeholder(raw)) = expr else {
            return ControlFlow::Continue(());
        };
        let Some(name) = name_of(raw) else {
            self.refused = Some(raw.clone());
            return ControlFlow::Break(());
        };
        if self.seen.insert(name.clone()) {
            self.names.push(name);
        }
        ControlFlow::Continue(())
    }
}

/// Rewrites every `:name` placeholder node to the native marker and
/// returns the values in marker order. `$n` numbers by first occurrence,
/// so a repeated name reuses its number and its value once; `?` repeats
/// the marker and the value.
pub(super) fn rewrite_markers(
    statement: &mut Statement,
    style: Marker,
    bound: &[(&str, &ParamValue)],
) -> Vec<ParamValue> {
    let mut rewriter = Rewriter {
        style,
        numbers: HashMap::new(),
        bound,
        values: Vec::new(),
    };
    // Method resolution prefers the immutable `Visit::visit`, so the
    // mutable pass is invoked as an explicit trait method.
    let _ = VisitMut::visit(statement, &mut rewriter);
    rewriter.values
}

struct Rewriter<'a> {
    style: Marker,
    numbers: HashMap<String, usize>,
    bound: &'a [(&'a str, &'a ParamValue)],
    values: Vec<ParamValue>,
}

impl VisitorMut for Rewriter<'_> {
    type Break = ();

    fn post_visit_expr(&mut self, expr: &mut Expr) -> ControlFlow<()> {
        let Expr::Value(Value::Placeholder(raw)) = expr else {
            return ControlFlow::Continue(());
        };
        let Some(name) = name_of(raw) else {
            return ControlFlow::Break(());
        };
        let value = self
            .bound
            .iter()
            .find(|(bound_name, _)| **bound_name == name)
            .map(|(_, value)| (*value).clone())
            .expect("placeholder names were proven equal to the bound names");
        match self.style {
            Marker::Dollar => {
                let number = match self.numbers.get(&name) {
                    Some(number) => *number,
                    None => {
                        let number = self.numbers.len() + 1;
                        self.numbers.insert(name, number);
                        self.values.push(value);
                        number
                    }
                };
                *raw = format!("${number}");
            }
            Marker::Question => {
                self.values.push(value);
                *raw = "?".to_owned();
            }
        }
        ControlFlow::Continue(())
    }
}
