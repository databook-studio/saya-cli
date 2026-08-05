use chrono::{DateTime, Duration, NaiveDate, NaiveTime, Utc};
use duckdb::types::{TimeUnit, Value as DuckValue, ValueRef};
use serde_json::Value;

pub(crate) fn json_value(value: ValueRef<'_>) -> Value {
    json_owned(value.to_owned())
}

fn json_owned(value: DuckValue) -> Value {
    match value {
        DuckValue::Null => Value::Null,
        DuckValue::Boolean(value) => Value::Bool(value),
        DuckValue::TinyInt(value) => Value::from(value),
        DuckValue::SmallInt(value) => Value::from(value),
        DuckValue::Int(value) => Value::from(value),
        DuckValue::BigInt(value) => Value::from(value),
        DuckValue::HugeInt(value) => Value::String(value.to_string()),
        DuckValue::UTinyInt(value) => Value::from(value),
        DuckValue::USmallInt(value) => Value::from(value),
        DuckValue::UInt(value) => Value::from(value),
        DuckValue::UBigInt(value) => Value::from(value),
        DuckValue::Float(value) => Value::from(f64::from(value)),
        DuckValue::Double(value) => Value::from(value),
        DuckValue::Decimal(value) => Value::String(value.to_string()),
        DuckValue::Text(value) | DuckValue::Enum(value) => Value::String(value),
        DuckValue::Blob(value) => {
            Value::String(value.iter().map(|byte| format!("{byte:02x}")).collect())
        }
        DuckValue::Date32(value) => Value::String(date(value)),
        DuckValue::Timestamp(unit, value) => Value::String(timestamp(unit, value)),
        DuckValue::Time64(unit, value) => Value::String(time(unit, value)),
        DuckValue::Interval {
            months,
            days,
            nanos,
        } => serde_json::json!({"months": months, "days": days, "nanos": nanos}),
        DuckValue::List(values) | DuckValue::Array(values) => {
            Value::Array(values.into_iter().map(json_owned).collect())
        }
        DuckValue::Struct(values) => Value::Object(
            values
                .iter()
                .map(|(key, value)| (key.clone(), json_owned(value.clone())))
                .collect(),
        ),
        DuckValue::Map(values) => Value::Array(
            values
                .iter()
                .map(|(key, value)| {
                    serde_json::json!([json_owned(key.clone()), json_owned(value.clone())])
                })
                .collect(),
        ),
        DuckValue::Union(value) => json_owned(*value),
    }
}

fn date(days: i32) -> String {
    NaiveDate::from_ymd_opt(1970, 1, 1)
        .and_then(|date| date.checked_add_signed(Duration::days(i64::from(days))))
        .map_or_else(|| days.to_string(), |date| date.to_string())
}

fn timestamp(unit: TimeUnit, value: i64) -> String {
    let nanos = nanos(unit, value);
    DateTime::<Utc>::from_timestamp(
        nanos.div_euclid(1_000_000_000) as i64,
        nanos.rem_euclid(1_000_000_000) as u32,
    )
    .map_or_else(|| value.to_string(), |time| time.to_rfc3339())
}

fn time(unit: TimeUnit, value: i64) -> String {
    let nanos = nanos(unit, value).rem_euclid(86_400_000_000_000) as u64;
    NaiveTime::from_num_seconds_from_midnight_opt(
        (nanos / 1_000_000_000) as u32,
        (nanos % 1_000_000_000) as u32,
    )
    .map_or_else(
        || value.to_string(),
        |time| time.format("%H:%M:%S%.f").to_string(),
    )
}

fn nanos(unit: TimeUnit, value: i64) -> i128 {
    match unit {
        TimeUnit::Second => i128::from(value) * 1_000_000_000,
        TimeUnit::Millisecond => i128::from(value) * 1_000_000,
        TimeUnit::Microsecond => i128::from(value) * 1_000,
        TimeUnit::Nanosecond => i128::from(value),
    }
}
