//! The Snowflake SQL API v2 encoding of validated bind values.
//!
//! `POST /api/v2/statements` takes `?` markers and a `bindings` object keyed
//! by 1-based position, where every value is a string: DATE as epoch
//! milliseconds, TIMESTAMP as epoch nanoseconds (Snowflake's documented
//! driver encoding). Values travel as JSON bindings only — never into the SQL
//! text — and no error message names one. The v2 API is single-statement, so
//! the 1-based keys always match the `?` marker order the safety layer emits.

use chrono::{DateTime, FixedOffset, NaiveDate, Utc};
use saya_types::ConnectionError;
use serde_json::{Value, json};

use crate::binds::BindValue;

#[cfg(test)]
#[path = "bindings_tests.rs"]
mod tests;

/// A null binds untyped: the official drivers send type `ANY` with a JSON
/// null value, which the server resolves from the statement's usage.
const NULL_TYPE: &str = "ANY";

/// Builds the `bindings` object for the v2 statement request: one entry per
/// `?` marker, keyed by its 1-based position.
pub(crate) fn bindings(values: &[BindValue]) -> Result<Value, ConnectionError> {
    let mut entries = serde_json::Map::new();
    for (index, value) in values.iter().enumerate() {
        entries.insert((index + 1).to_string(), entry(value)?);
    }
    Ok(Value::Object(entries))
}

fn entry(value: &BindValue) -> Result<Value, ConnectionError> {
    Ok(match value {
        BindValue::Null(_) => json!({"type": NULL_TYPE, "value": Value::Null}),
        BindValue::Str(text) => json!({"type": "TEXT", "value": text}),
        BindValue::Int(int) => json!({"type": "FIXED", "value": int.to_string()}),
        BindValue::Bool(flag) => json!({"type": "BOOLEAN", "value": bool_text(*flag)}),
        // FIXED carries the value as given; the server parses the string.
        BindValue::Decimal { text, .. } => json!({"type": "FIXED", "value": text}),
        BindValue::Date(date) => json!({"type": "DATE", "value": epoch_ms(*date).to_string()}),
        BindValue::Timestamp { value, .. } => {
            json!({"type": "TIMESTAMP_TZ", "value": timestamp_tz(*value)?})
        }
    })
}

fn bool_text(flag: bool) -> &'static str {
    if flag { "true" } else { "false" }
}

/// DATE binds as the milliseconds from the Unix epoch to midnight UTC of the
/// validated date.
fn epoch_ms(date: NaiveDate) -> i64 {
    date.and_hms_opt(0, 0, 0)
        .expect("midnight of a calendar date exists")
        .and_utc()
        .timestamp_millis()
}

/// TIMESTAMP_TZ binds as the nanoseconds from the Unix epoch, a space, and
/// the offset in minutes east of UTC shifted by 1440 — UTC itself encodes as
/// 1440, +02:00 as 1560, -08:00 as 0960, exactly as the reference drivers
/// encode the offset piggyback.
fn timestamp_tz(value: DateTime<FixedOffset>) -> Result<String, ConnectionError> {
    let nanos = value
        .with_timezone(&Utc)
        .timestamp_nanos_opt()
        .ok_or_else(|| {
            ConnectionError::query_failed(
                "a bound timestamp parameter is outside the range the Snowflake API can encode",
            )
        })?;
    let minutes = value.offset().local_minus_utc() / 60 + 1440;
    Ok(format!("{nanos} {minutes:04}"))
}
