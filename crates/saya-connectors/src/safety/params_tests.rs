//! Tests for parameter-aware preparation (`params.rs`): a query's
//! placeholders are proven to be exactly the bound names and rewritten in
//! the AST to the dialect's native markers — never substituted into text.

use super::prepare_sqlite_sql;
use saya_types::{BoundParam, ParamValue, SqlDialect};

use super::params::{prepare_with_params, sql_placeholders};

fn bound(name: &str, value: ParamValue) -> BoundParam {
    BoundParam {
        name: name.to_owned(),
        value,
    }
}

fn city() -> BoundParam {
    bound("city", ParamValue::String("paris".to_owned()))
}

fn floor() -> BoundParam {
    bound("floor", ParamValue::Integer(5))
}

fn dashed(query: &str) -> String {
    query.replace(' ', "").to_ascii_lowercase()
}

mod binding;
mod ordering;
mod sentinel;
