//! The shared bridge from validated [`ParamValue`]s to engine-native binds.
//!
//! Values arrive grammar-checked from `saya-types`, but a deserialised
//! request can carry any string, so decimal, date and timestamp text is
//! re-checked here before it reaches an engine. Nothing interpolates: values
//! travel to sqlx's bind API only, and no error message names a value.

use std::fmt;

use bigdecimal::BigDecimal;
use chrono::{DateTime, FixedOffset, NaiveDate};
use saya_types::{ConnectionError, ParamValue};

#[cfg(test)]
#[path = "binds_tests.rs"]
mod tests;

/// A bound value in the concrete types the sqlx engines encode natively.
/// `Decimal` and `Timestamp` keep the validated text beside the parsed value:
/// engines that compare numbers and instants bind the parsed value, while
/// text engines (SQLite) bind the exact text the user validated.
///
/// Like [`ParamValue`], values are sensitive: the [`fmt::Debug`] impl prints
/// the variant only, never a value.
#[derive(Clone, PartialEq)]
pub(crate) enum BindValue {
    Null,
    Str(String),
    Int(i64),
    Bool(bool),
    Decimal {
        value: BigDecimal,
        text: String,
    },
    Date(NaiveDate),
    Timestamp {
        value: DateTime<FixedOffset>,
        text: String,
    },
}

impl fmt::Debug for BindValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Null => "Null",
            Self::Str(_) => "Str(_)",
            Self::Int(_) => "Int(_)",
            Self::Bool(_) => "Bool(_)",
            Self::Decimal { .. } => "Decimal { .. }",
            Self::Date(_) => "Date(_)",
            Self::Timestamp { .. } => "Timestamp { .. }",
        })
    }
}

/// Converts validated values to engine-native types, in marker order.
pub(crate) fn parse_bind_values(values: &[ParamValue]) -> Result<Vec<BindValue>, ConnectionError> {
    values.iter().map(parse_bind_value).collect()
}

fn parse_bind_value(value: &ParamValue) -> Result<BindValue, ConnectionError> {
    Ok(match value {
        ParamValue::Null => BindValue::Null,
        ParamValue::String(text) => BindValue::Str(text.clone()),
        ParamValue::Integer(int) => BindValue::Int(*int),
        ParamValue::Boolean(flag) => BindValue::Bool(*flag),
        ParamValue::Decimal(text) => BindValue::Decimal {
            value: text.parse().map_err(|_| {
                ConnectionError::query_failed(
                    "a bound decimal parameter is not a valid decimal number",
                )
            })?,
            text: text.clone(),
        },
        ParamValue::Date(text) => {
            BindValue::Date(NaiveDate::parse_from_str(text, "%Y-%m-%d").map_err(|_| {
                ConnectionError::query_failed(
                    "a bound date parameter is not a valid YYYY-MM-DD date",
                )
            })?)
        }
        ParamValue::Timestamp(text) => BindValue::Timestamp {
            value: DateTime::parse_from_rfc3339(text).map_err(|_| {
                ConnectionError::query_failed(
                    "a bound timestamp parameter is not a valid RFC 3339 timestamp",
                )
            })?,
            text: text.clone(),
        },
    })
}
