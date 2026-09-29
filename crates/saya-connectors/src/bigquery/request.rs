use bigdecimal::BigDecimal;
use chrono::{SecondsFormat, Utc};
use saya_types::{ConnectionError, QueryResult};
use serde_json::Value;

use crate::binds::BindValue;
use crate::common::{MAX_RESULT_BYTES, cap_cell, value_bytes};

/// BigQuery's NUMERIC is precision ≤ 38 and scale ≤ 9; BIGNUMERIC widens both.
const NUMERIC_MAX_PRECISION: usize = 38;
const NUMERIC_MAX_SCALE: usize = 9;
const BIGNUMERIC_MAX_PRECISION: usize = 77;
const BIGNUMERIC_MAX_SCALE: usize = 38;

/// The `jobs.query` request body. `maxResults` is set one above the row cap so
/// a result that fills the cap but has more rows reports `pageToken` and is
/// marked truncated, while `maximumBytesBilled` is the server-side cost bound
/// that fails the job if the query scans more than the configured allowance.
/// `parameters` carries the positional `queryParameters` — `None` for a
/// parameter-free query, which keeps this body byte-for-byte as before.
pub(crate) fn query_body(
    sql: &str,
    max_rows: usize,
    cap: u64,
    location: Option<&str>,
    parameters: Option<&[Value]>,
) -> Value {
    let mut body = serde_json::json!({
        "query": sql,
        "useLegacySql": false,
        "maxResults": max_rows.saturating_add(1),
        "maximumBytesBilled": cap.to_string(),
    });
    if let Some(location) = location {
        body["location"] = Value::String(location.into());
    }
    if let Some(parameters) = parameters {
        body["parameterMode"] = Value::String("POSITIONAL".into());
        body["queryParameters"] = Value::Array(parameters.to_vec());
    }
    body
}

/// The `jobs.insert` body for a dry-run. BigQuery estimates the bytes the
/// query would scan without running it, which lets the connector refuse an
/// over-budget query before it executes. `parameters` must carry the same
/// positional parameters the query itself sends, or the estimate is computed
/// against a different statement than the one that will run.
pub(crate) fn dry_run_body(
    sql: &str,
    cap: u64,
    location: Option<&str>,
    parameters: Option<&[Value]>,
) -> Value {
    let mut query = serde_json::json!({
        "query": sql,
        "useLegacySql": false,
        "maximumBytesBilled": cap.to_string(),
    });
    if let Some(location) = location {
        query["location"] = Value::String(location.into());
    }
    if let Some(parameters) = parameters {
        query["parameterMode"] = Value::String("POSITIONAL".into());
        query["queryParameters"] = Value::Array(parameters.to_vec());
    }
    serde_json::json!({
        "configuration": {
            "dryRun": true,
            "query": query,
        }
    })
}

/// Encodes validated values as BigQuery's positional `queryParameters`: one
/// entry per `?` marker with a concrete `parameterType` and a string
/// `parameterValue` — values travel in the body only, never into the SQL
/// text, and no error message names one.
pub(crate) fn query_parameters(values: &[BindValue]) -> Result<Vec<Value>, ConnectionError> {
    values.iter().map(query_parameter).collect()
}

fn query_parameter(value: &BindValue) -> Result<Value, ConnectionError> {
    let kind = match value {
        BindValue::Null => {
            return Err(ConnectionError::query_failed(
                "a null parameter cannot be bound on BigQuery: the service requires a \
                 concrete parameter type for null values",
            ));
        }
        BindValue::Str(_) => "STRING",
        BindValue::Int(_) => "INT64",
        BindValue::Bool(_) => "BOOL",
        BindValue::Decimal { value, .. } => numeric_kind(value)?,
        BindValue::Date(_) => "DATE",
        BindValue::Timestamp { .. } => "TIMESTAMP",
    };
    Ok(serde_json::json!({
        "parameterType": {"type": kind},
        "parameterValue": {"value": value_text(value)},
    }))
}

/// Picks NUMERIC for a value inside its 38-digit/9-scale bound and
/// BIGNUMERIC otherwise, refusing what even BIGNUMERIC cannot carry so an
/// oversized value fails here instead of echoing from the server.
fn numeric_kind(value: &BigDecimal) -> Result<&'static str, ConnectionError> {
    let scale = value.fractional_digit_count().max(0) as usize;
    let (digits, _) = value.with_scale(scale as i64).into_bigint_and_exponent();
    // The magnitude carries the precision (77 digits outgrows i128), so it is
    // counted as text rather than converted.
    let precision = digits.magnitude().to_string().len().max(scale).max(1);
    let refusal = || {
        ConnectionError::query_failed(
            "a bound decimal parameter does not fit BigQuery's BIGNUMERIC precision",
        )
    };
    if precision <= NUMERIC_MAX_PRECISION && scale <= NUMERIC_MAX_SCALE {
        return Ok("NUMERIC");
    }
    if precision <= BIGNUMERIC_MAX_PRECISION && scale <= BIGNUMERIC_MAX_SCALE {
        return Ok("BIGNUMERIC");
    }
    Err(refusal())
}

/// The string a BigQuery `parameterValue` carries for each validated type:
/// text and decimals exactly as given, a date in canonical YYYY-MM-DD, and a
/// timestamp as the instant's canonical UTC RFC 3339 form.
fn value_text(value: &BindValue) -> String {
    match value {
        BindValue::Null => String::new(),
        BindValue::Str(text) => text.clone(),
        BindValue::Int(int) => int.to_string(),
        BindValue::Bool(flag) => flag.to_string(),
        BindValue::Decimal { text, .. } => text.clone(),
        BindValue::Date(date) => date.format("%Y-%m-%d").to_string(),
        BindValue::Timestamp { value, .. } => value
            .with_timezone(&Utc)
            .to_rfc3339_opts(SecondsFormat::AutoSi, true),
    }
}

/// Turns a `jobs.query` response into a bounded `QueryResult`. `schema.fields`
/// names the columns in select order; `rows[].f[].v` carries the cell values.
/// Rows are accumulated up to the row cap and the shared byte budget, setting
/// `truncated` when either bound stops the loop or when a `pageToken` reports
/// more rows remain.
pub(crate) fn parse_result(value: Value, max_rows: usize, original_sql: String) -> QueryResult {
    let columns = value
        .get("schema")
        .and_then(|s| s.get("fields"))
        .and_then(Value::as_array)
        .map(|fields| {
            fields
                .iter()
                .map(|field| {
                    field
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string()
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let rows = value.get("rows").and_then(Value::as_array).cloned();
    let page_token = value.get("pageToken").is_some();

    let mut collected = Vec::new();
    let mut result_bytes = 0;
    let mut truncated = page_token;
    if let Some(rows) = rows {
        for row in rows {
            if collected.len() == max_rows {
                truncated = true;
                break;
            }
            let cells = row
                .get("f")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            // BigQuery rows are positional: one `f` entry per column. Pad a
            // short row with null so every result row matches the column count
            // rather than silently dropping trailing columns.
            let values: Vec<Value> = (0..columns.len())
                .map(|index| {
                    cells
                        .get(index)
                        .and_then(|cell| cell.get("v").cloned())
                        .unwrap_or(Value::Null)
                })
                .map(cap_cell)
                .collect();
            for cell in &values {
                result_bytes += value_bytes(cell);
            }
            collected.push(Value::Array(values));
            if result_bytes > MAX_RESULT_BYTES {
                truncated = true;
                break;
            }
        }
    }
    QueryResult {
        row_count: collected.len(),
        columns,
        rows: collected,
        truncated,
        executed_sql: original_sql,
    }
}

#[cfg(test)]
#[path = "request_tests.rs"]
mod tests;
