//! The `join_check` tool's body: build a fan-out probe for the statement, run
//! the two `COUNT(*)` statements through the same read-only path as
//! `bounded_sql_query`, and report whether the join multiplied or dropped rows
//! — without ever returning a cell value.
//!
//! The probe is built by [`saya_connectors::fanout_probe`], which emits two
//! `COUNT(*)` statements: one over the full join and one over the base table
//! alone. Their disagreement proves the join changed the row count. When no
//! sound probe can be built, the tool says so (`applicable: false`) rather than
//! guessing.

use serde_json::{Value, json};

use saya_agent::ToolError;
use saya_connectors::{
    prepare_bigquery_sql, prepare_clickhouse_sql, prepare_duckdb_sql, prepare_mysql_sql,
    prepare_postgres_sql, prepare_snowflake_sql, prepare_sqlite_sql,
};
use saya_types::SqlDialect;

use super::DatabaseTools;
use crate::connection::ConnectionEntry;

impl DatabaseTools {
    /// Builds a fan-out probe for `sql`, runs the two COUNT statements through
    /// the bounded read-only path, and reports whether the join fanned out or
    /// dropped rows. When no sound probe can be built, returns `applicable:
    /// false` with a reason — never a guess.
    pub(super) async fn join_check(
        &self,
        entry: &ConnectionEntry,
        sql: &str,
    ) -> Result<Value, ToolError> {
        self.detect_and_record_overrides(sql, entry.dialect);
        ensure_read_only(sql, entry.dialect)?;

        let Some(probe) = saya_connectors::fanout_probe(sql, entry.dialect) else {
            return Ok(json!({
                "applicable": false,
                "reason": "no sound fan-out probe could be built; the probe requires a single \
                    SELECT with one plain base table, at least one JOIN to another plain table, \
                    a top-level SUM/AVG/COUNT (without DISTINCT), and a WHERE that touches only \
                    the base table"
            }));
        };

        let joined_result = crate::agent::state_tools::query(
            entry.connector.as_ref(),
            &probe.joined_rows,
            self.max_rows,
            self.state_db.as_ref(),
            entry.profile_id.as_deref(),
        )
        .await?;
        let base_result = crate::agent::state_tools::query(
            entry.connector.as_ref(),
            &probe.base_rows,
            self.max_rows,
            self.state_db.as_ref(),
            entry.profile_id.as_deref(),
        )
        .await?;

        let (Some(joined), Some(base)) = (single_count(&joined_result), single_count(&base_result))
        else {
            return Ok(json!({
                "applicable": false,
                "reason": "the probe statements did not each return a single numeric count"
            }));
        };

        Ok(json!({
            "applicable": true,
            "joined_rows": joined,
            "base_rows": base,
            "fanned_out": joined > base,
            "dropped_rows": joined < base,
        }))
    }
}

/// Validates that `sql` is a read-only statement the safety layer would allow,
/// dispatching to the per-dialect `prepare_*` function. This mirrors the check
/// the connector applies at execution time, so a write statement is refused here
/// by the same read-only policy — before any probe is built.
fn ensure_read_only(sql: &str, dialect: SqlDialect) -> Result<(), ToolError> {
    let prepared = match dialect {
        SqlDialect::Postgres => prepare_postgres_sql(sql, 1),
        SqlDialect::Mysql => prepare_mysql_sql(sql, 1),
        SqlDialect::DuckDb => prepare_duckdb_sql(sql, 1),
        SqlDialect::Snowflake => prepare_snowflake_sql(sql, 1),
        SqlDialect::Sqlite => prepare_sqlite_sql(sql, 1),
        SqlDialect::ClickHouse => prepare_clickhouse_sql(sql, 1),
        SqlDialect::BigQuery => prepare_bigquery_sql(sql, 1),
        _ => prepare_postgres_sql(sql, 1),
    };
    prepared
        .map(|_| ())
        .map_err(|e| ToolError::QueryFailedDetail(e.to_string()))
}

/// Extracts a single integral count from a serialized `SELECT COUNT(*) AS n`
/// result. Anything that is not exactly one row, one column, one numeric cell
/// is `None` — never guessed.
fn single_count(result: &Value) -> Option<i64> {
    let rows = result.get("rows").and_then(Value::as_array)?;
    let columns = result.get("columns").and_then(Value::as_array)?;
    if rows.len() != 1 || columns.len() != 1 {
        return None;
    }
    let cell = match &rows[0] {
        Value::Array(cells) if cells.len() == 1 => &cells[0],
        Value::Array(_) => return None,
        other => other,
    };
    match cell {
        Value::Number(num) => num
            .as_i64()
            .or_else(|| num.as_f64().filter(|f| f.fract() == 0.0).map(|f| f as i64)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_count_extracts_an_integer_cell() {
        let result = json!({
            "columns": ["n"],
            "rows": [[42]],
            "row_count": 1,
            "truncated": false,
        });
        assert_eq!(single_count(&result), Some(42));
    }

    #[test]
    fn single_count_extracts_a_whole_float_cell() {
        let result = json!({
            "columns": ["n"],
            "rows": [[100.0]],
            "row_count": 1,
            "truncated": false,
        });
        assert_eq!(single_count(&result), Some(100));
    }

    #[test]
    fn single_count_rejects_multiple_rows() {
        let result = json!({
            "columns": ["n"],
            "rows": [[1], [2]],
            "row_count": 2,
            "truncated": false,
        });
        assert_eq!(single_count(&result), None);
    }

    #[test]
    fn single_count_rejects_a_string_cell() {
        let result = json!({
            "columns": ["n"],
            "rows": [["not a number"]],
            "row_count": 1,
            "truncated": false,
        });
        assert_eq!(single_count(&result), None);
    }
}
