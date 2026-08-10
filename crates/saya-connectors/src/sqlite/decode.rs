use serde_json::Value;
use sqlx::{Row, TypeInfo, ValueRef, sqlite::SqliteRow};

pub(crate) fn json_value(row: &SqliteRow, index: usize) -> Result<Value, sqlx::Error> {
    let raw = row.try_get_raw(index)?;
    if raw.is_null() {
        return Ok(Value::Null);
    }
    let type_info = raw.type_info();
    let type_name = type_info.name();
    match type_name {
        "NULL" => Ok(Value::Null),
        "INTEGER" => row.try_get::<i64, _>(index).map(Value::from),
        "REAL" => row.try_get::<f64, _>(index).map(Value::from),
        "TEXT" => row.try_get::<String, _>(index).map(Value::String),
        "BLOB" => row
            .try_get::<Vec<u8>, _>(index)
            .map(|v| Value::String(crate::common::bytes_to_hex(&v))),
        _ => Ok(row
            .try_get::<Vec<u8>, _>(index)
            .map(|bytes| Value::String(String::from_utf8_lossy(&bytes).into_owned()))
            .unwrap_or(Value::Null)),
    }
}
