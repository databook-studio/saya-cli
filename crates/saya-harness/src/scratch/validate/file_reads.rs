//! The file-reader walk: a visitor that refuses every DuckDB reader of
//! data outside the scratch file — local files, remote URLs, other
//! engines' files — matched per identifier part, as a category.

use std::ops::ControlFlow;

use sqlparser::ast::{Expr, ObjectName, TableFactor, Visitor};

use super::types::ScratchRejection;

/// Every DuckDB reader of data outside the scratch file — local files, remote
/// URLs, other engines' files — starts with one of these prefixes: `read_csv`,
/// `read_parquet`, `read_json`, `http_get`, … Matched per identifier part
/// (lowercased, unquoted) so qualification or requoting cannot hide the name.
const FILE_READ_PREFIXES: &[&str] = &["read_", "http_"];

/// The readers that carry no prefix: table functions over files, other
/// engines, or the filesystem.
const FILE_READ_FUNCTIONS: &[&str] = &[
    "glob",
    "metadata",
    "parquet_scan",
    "parquet_metadata",
    "parquet_schema",
    "parquet_file_metadata",
    "parquet_kv_metadata",
    "sqlite_scan",
    "postgres_scan",
    "mysql_scan",
    "arrow",
    "arrow_scan",
    "iceberg_scan",
    "delta_scan",
];

pub(super) struct Guard {
    pub(super) denied: Option<ScratchRejection>,
}

impl Visitor for Guard {
    type Break = ();

    fn pre_visit_expr(&mut self, expr: &Expr) -> ControlFlow<Self::Break> {
        if let Expr::Function(function) = expr
            && let Some(denied) = denied_file_name(&function.name)
        {
            self.denied = Some(denied);
            return ControlFlow::Break(());
        }
        ControlFlow::Continue(())
    }

    fn pre_visit_table_factor(&mut self, factor: &TableFactor) -> ControlFlow<Self::Break> {
        // A table function in `FROM` (`read_csv('x.csv')`) carries its name in
        // `Table { args }`; a `LATERAL fn(...)` in `Function`. Plain relations
        // get the same per-part check — stricter than the connector's
        // whole-name posture, and safe here because every relation inside
        // scratch is one the model itself created. Derived tables, UNNEST and
        // JSON_TABLE carry no name of their own; the visitor recurses into
        // their subqueries and expressions, which these checks cover.
        let (TableFactor::Table { name, .. } | TableFactor::Function { name, .. }) = factor else {
            return ControlFlow::Continue(());
        };
        if let Some(denied) = denied_file_name(name) {
            self.denied = Some(denied);
            return ControlFlow::Break(());
        }
        ControlFlow::Continue(())
    }
}

/// `Some(refusal)` when any identifier part of the name is a file reader.
/// A single-quoted identifier is DuckDB's file-read shorthand (`FROM 'data.csv'`)
/// and is refused as the file path it is.
fn denied_file_name(name: &ObjectName) -> Option<ScratchRejection> {
    name.0.iter().find_map(|ident| {
        if ident.quote_style == Some('\'') {
            return Some(ScratchRejection::FilePathRelation(ident.value.clone()));
        }
        let part = ident.value.to_ascii_lowercase();
        if FILE_READ_PREFIXES
            .iter()
            .any(|prefix| part.starts_with(prefix))
            || FILE_READ_FUNCTIONS.contains(&part.as_str())
        {
            return Some(ScratchRejection::FileFunction(part));
        }
        None
    })
}
