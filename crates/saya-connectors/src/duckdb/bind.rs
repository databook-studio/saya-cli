//! The DuckDB-native encoding of validated bind values.
//!
//! Values arrive grammar-checked from `saya-types` and re-checked in
//! `crate::binds`; nothing interpolates: values travel to DuckDB's bind API
//! as native `duckdb::types::Value`s only, and no error message names a
//! value.

use crate::binds::BindValue;
use bigdecimal::{BigDecimal, ToPrimitive};
use chrono::{NaiveDate, Utc};
use duckdb::types::{Decimal as DuckDecimal, TimeUnit, Value};
use saya_types::ConnectionError;

#[cfg(test)]
#[path = "bind_tests.rs"]
mod tests;

/// The widest DECIMAL DuckDB can carry.
const MAX_DECIMAL_WIDTH: u8 = 38;

/// Converts validated values to DuckDB's native bind types, in marker order.
/// Decimals bind as a DuckDB `DECIMAL(width, scale)` payload — text binding
/// was probed and rejected: DuckDB's VARCHAR→DECIMAL cast rounds to the
/// compared column's scale, silently changing the comparison. Dates bind as
/// `Date32` (days since the Unix epoch) and timestamps as microseconds since
/// the epoch's UTC instant.
pub(crate) fn native_values(values: &[BindValue]) -> Result<Vec<Value>, ConnectionError> {
    values.iter().map(native_value).collect()
}

fn native_value(value: &BindValue) -> Result<Value, ConnectionError> {
    Ok(match value {
        BindValue::Null(_) => Value::Null,
        BindValue::Str(text) => Value::Text(text.clone()),
        BindValue::Int(int) => Value::BigInt(*int),
        BindValue::Bool(flag) => Value::Boolean(*flag),
        BindValue::Decimal { value, .. } => Value::Decimal(decimal(value)?),
        BindValue::Date(date) => Value::Date32(epoch_days(*date)),
        BindValue::Timestamp { value, .. } => Value::Timestamp(
            TimeUnit::Microsecond,
            value.with_timezone(&Utc).timestamp_micros(),
        ),
    })
}

/// Encodes the validated decimal as DuckDB's `DECIMAL(width, scale)` scaled
/// payload. DuckDB caps decimals at 38 digits, so a wider value refuses —
/// with an error this layer builds itself, naming no value.
fn decimal(value: &BigDecimal) -> Result<DuckDecimal, ConnectionError> {
    let refusal = || {
        ConnectionError::query_failed(
            "a bound decimal parameter does not fit DuckDB's 38-digit decimal precision",
        )
    };
    let scale = value.fractional_digit_count().max(0);
    if scale > i64::from(MAX_DECIMAL_WIDTH) {
        return Err(refusal());
    }
    // Raising the scale to a non-negative value only appends zeros, so the
    // scaled integer keeps the value exactly; a negative-scale value like
    // "1e5" becomes its plain digits.
    let (digits, _) = value.with_scale(scale).into_bigint_and_exponent();
    let payload = digits.to_i128().ok_or_else(refusal)?;
    let width = payload
        .unsigned_abs()
        .to_string()
        .len()
        .max(scale as usize)
        .max(1);
    if width > usize::from(MAX_DECIMAL_WIDTH) {
        return Err(refusal());
    }
    DuckDecimal::new(width as u8, scale as u8, payload).map_err(|_| refusal())
}

fn epoch_days(date: NaiveDate) -> i32 {
    let epoch = NaiveDate::from_ymd_opt(1970, 1, 1).unwrap();
    date.signed_duration_since(epoch).num_days() as i32
}
