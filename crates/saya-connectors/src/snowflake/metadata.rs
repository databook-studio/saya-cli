use std::collections::BTreeMap;

use saya_types::{Column, ConnectionError, Database, Schema, SchemaTree, Table};
use serde_json::Value;

use super::{client::SnowflakeConnector, errors};
use crate::DatabaseConnector;

const PAGE: usize = 5000;
const MAX_COLUMNS: usize = 200_000;

pub(crate) async fn schema(connector: &SnowflakeConnector) -> Result<SchemaTree, ConnectionError> {
    let database = connector
        .context
        .database
        .as_deref()
        .ok_or_else(errors::schema)?;
    let schema = connector
        .context
        .schema
        .as_deref()
        .ok_or_else(errors::schema)?;

    let mut tables = BTreeMap::<String, Vec<Column>>::new();
    let mut total_columns: usize = 0;
    let mut offset: usize = 0;

    loop {
        let sql = format!(
            "SELECT table_catalog, table_schema, table_name, column_name, data_type, is_nullable \
             FROM {}.INFORMATION_SCHEMA.COLUMNS \
             WHERE table_schema = '{}' AND table_schema <> 'INFORMATION_SCHEMA' \
             ORDER BY table_name, ordinal_position \
             LIMIT {} OFFSET {}",
            quote(database),
            literal(schema),
            PAGE,
            offset
        );

        let output = connector
            .execute(saya_types::QueryRequest::new(sql, PAGE))
            .await
            .map_err(|_| errors::schema())?;

        let fetched_len = output.rows.len();
        if fetched_len == 0 {
            break;
        }

        process_rows(output.rows, &mut tables, &mut total_columns, MAX_COLUMNS)?;

        if fetched_len < PAGE {
            break;
        }

        offset = match offset.checked_add(PAGE) {
            Some(next) => next,
            None => break,
        };
    }

    Ok(SchemaTree {
        databases: vec![Database {
            name: database.into(),
            schemas: vec![Schema {
                name: schema.into(),
                tables: tables
                    .into_iter()
                    .map(|(name, columns)| Table { name, columns })
                    .collect(),
            }],
        }],
    })
}

fn process_rows(
    rows: Vec<Value>,
    tables: &mut BTreeMap<String, Vec<Column>>,
    total_columns: &mut usize,
    max_columns: usize,
) -> Result<(), ConnectionError> {
    if total_columns.saturating_add(rows.len()) > max_columns {
        return Err(ConnectionError::SchemaFailed(
            "Snowflake schema is too large to enumerate completely; narrow the database/schema"
                .into(),
        ));
    }
    for row in rows {
        let values = row.as_array().ok_or_else(errors::schema)?;
        if values.len() != 6 {
            return Err(errors::schema());
        }
        let name = text(&values[2])?;
        tables.entry(name).or_default().push(Column {
            name: text(&values[3])?,
            data_type: text(&values[4])?,
            nullable: text(&values[5])?.eq_ignore_ascii_case("YES"),
        });
        *total_columns += 1;
    }
    Ok(())
}

fn text(value: &Value) -> Result<String, ConnectionError> {
    value.as_str().map(str::to_owned).ok_or_else(errors::schema)
}

fn quote(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn literal(value: &str) -> String {
    value.replace('\'', "''")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_row(table: &str, col: &str, dtype: &str, nullable: &str) -> Value {
        json!(["DB", "SCH", table, col, dtype, nullable])
    }

    #[test]
    fn test_process_rows_grouping() {
        let mut tables = BTreeMap::new();
        let mut total = 0;
        let rows = vec![
            make_row("T1", "C1", "INT", "NO"),
            make_row("T1", "C2", "VARCHAR", "YES"),
            make_row("T2", "C1", "BOOLEAN", "NO"),
        ];

        process_rows(rows, &mut tables, &mut total, 100).unwrap();

        assert_eq!(total, 3);
        assert_eq!(tables.len(), 2);

        let t1 = &tables["T1"];
        assert_eq!(t1.len(), 2);
        assert_eq!(t1[0].name, "C1");
        assert_eq!(t1[0].data_type, "INT");
        assert!(!t1[0].nullable);
        assert_eq!(t1[1].name, "C2");
        assert_eq!(t1[1].data_type, "VARCHAR");
        assert!(t1[1].nullable);

        let t2 = &tables["T2"];
        assert_eq!(t2.len(), 1);
        assert_eq!(t2[0].name, "C1");
        assert!(!t2[0].nullable);
    }

    #[test]
    fn test_process_rows_max_columns_exceeded() {
        let mut tables = BTreeMap::new();
        let mut total = 0;
        let rows_p1 = vec![
            make_row("T1", "C1", "INT", "NO"),
            make_row("T1", "C2", "INT", "NO"),
        ];

        process_rows(rows_p1, &mut tables, &mut total, 3).unwrap();
        assert_eq!(total, 2);

        let rows_p2 = vec![
            make_row("T1", "C3", "INT", "NO"),
            make_row("T1", "C4", "INT", "NO"),
        ];

        let err = process_rows(rows_p2, &mut tables, &mut total, 3).unwrap_err();
        match err {
            ConnectionError::SchemaFailed(msg) => {
                assert!(msg.contains("Snowflake schema is too large to enumerate completely"));
            }
            _ => panic!("unexpected error type"),
        }
    }

    #[test]
    fn test_process_rows_malformed_row() {
        let mut tables = BTreeMap::new();
        let mut total = 0;
        let rows = vec![json!(["DB", "SCH", "T1"])];

        assert!(process_rows(rows, &mut tables, &mut total, 100).is_err());
    }
}
