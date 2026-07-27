use bigdecimal::BigDecimal;
use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use serde_json::Value;
use sqlx::{Row, TypeInfo, ValueRef, postgres::PgRow};

pub(crate) fn json_value(row: &PgRow, index: usize) -> Result<Value, sqlx::Error> {
    let raw = row.try_get_raw(index)?;
    if raw.is_null() {
        return Ok(Value::Null);
    }
    match raw.type_info().name() {
        "BOOL" => row.try_get::<bool, _>(index).map(Value::Bool),
        "INT2" => row
            .try_get::<i16, _>(index)
            .map(|v| Value::from(i64::from(v))),
        "INT4" => row
            .try_get::<i32, _>(index)
            .map(|v| Value::from(i64::from(v))),
        "INT8" => row.try_get::<i64, _>(index).map(Value::from),
        "OID" => row
            .try_get::<i32, _>(index)
            .map(|value| Value::from(value as u32)),
        "FLOAT4" => row
            .try_get::<f32, _>(index)
            .map(|v| Value::from(f64::from(v))),
        "FLOAT8" => row.try_get::<f64, _>(index).map(Value::from),
        "NUMERIC" => row
            .try_get::<BigDecimal, _>(index)
            .map(|value| Value::String(value.to_string())),
        "JSON" | "JSONB" => row.try_get(index),
        "DATE" => row
            .try_get::<NaiveDate, _>(index)
            .map(|v| Value::String(v.to_string())),
        "TIME" => row
            .try_get::<NaiveTime, _>(index)
            .map(|v| Value::String(v.to_string())),
        "TIMESTAMP" => row
            .try_get::<NaiveDateTime, _>(index)
            .map(|v| Value::String(v.to_string())),
        "TIMESTAMPTZ" => row
            .try_get::<DateTime<Utc>, _>(index)
            .map(|v| Value::String(v.to_rfc3339())),
        "UUID" => row
            .try_get::<uuid::Uuid, _>(index)
            .map(|v| Value::String(v.to_string())),
        "BYTEA" => row
            .try_get::<Vec<u8>, _>(index)
            .map(|v| Value::String(hex(&v))),
        _ => row.try_get::<String, _>(index).map(Value::String),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
