use bigdecimal::BigDecimal;
use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use serde_json::Value;
use sqlx::{Row, TypeInfo, ValueRef, mysql::MySqlRow};

pub(crate) fn json_value(row: &MySqlRow, index: usize) -> Result<Value, sqlx::Error> {
    let raw = row.try_get_raw(index)?;
    if raw.is_null() {
        return Ok(Value::Null);
    }
    match raw.type_info().name() {
        "BOOL" | "BOOLEAN" => row.try_get::<bool, _>(index).map(Value::Bool),
        "TINYINT" | "SMALLINT" | "MEDIUMINT" | "INT" | "INTEGER" | "BIGINT" => {
            row.try_get::<i64, _>(index).map(Value::from)
        }
        "TINYINT UNSIGNED" | "SMALLINT UNSIGNED" | "MEDIUMINT UNSIGNED" | "INT UNSIGNED"
        | "BIGINT UNSIGNED" => row.try_get::<u64, _>(index).map(Value::from),
        "FLOAT" | "DOUBLE" => row.try_get::<f64, _>(index).map(Value::from),
        "DECIMAL" | "NEWDECIMAL" => row
            .try_get::<BigDecimal, _>(index)
            .map(|value| Value::String(value.to_string())),
        "JSON" => row.try_get(index),
        "DATE" => row
            .try_get::<NaiveDate, _>(index)
            .map(|value| Value::String(value.to_string())),
        "TIME" => row
            .try_get::<NaiveTime, _>(index)
            .map(|value| Value::String(value.to_string())),
        "DATETIME" => row
            .try_get::<NaiveDateTime, _>(index)
            .map(|value| Value::String(value.to_string())),
        "TIMESTAMP" => row
            .try_get::<DateTime<Utc>, _>(index)
            .map(|value| Value::String(value.naive_utc().to_string())),
        "YEAR" | "BIT" => row.try_get::<u64, _>(index).map(Value::from),
        "BLOB" | "TINYBLOB" | "MEDIUMBLOB" | "LONGBLOB" | "BINARY" | "VARBINARY" => row
            .try_get::<Vec<u8>, _>(index)
            .map(|bytes| Value::String(hex(&bytes))),
        _ => row.try_get::<String, _>(index).map(Value::String),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
