use std::collections::BTreeMap;

use saya_types::{
    Column, ConnectionError, Database, MAX_SCHEMA_COLUMNS, MAX_SCHEMA_TABLES, Schema, SchemaTree,
    Table,
};

use super::BigQueryConnector;
use super::errors;
use crate::DatabaseConnector;

/// Upper bound on the column rows fetched in one pass. A real dataset stays
/// well under this; the cap fails closed instead of growing without limit.
const PAGE: usize = 5_000;
const MAX_COLUMNS: usize = MAX_SCHEMA_COLUMNS;

/// Discovers one dataset's tables and columns. Both `INFORMATION_SCHEMA.TABLES`
/// and `INFORMATION_SCHEMA.COLUMNS` are read per dataset: TABLES enumerates
/// every table (including those with no discovered columns), and COLUMNS
/// attaches each column in declared order.
///
/// Foreign keys are deliberately not extracted. BigQuery does not enforce
/// foreign keys, and its unenforced constraint metadata cannot be validated
/// without live credentials; reporting none is the honest state.
pub(crate) async fn schema(connector: &BigQueryConnector) -> Result<SchemaTree, ConnectionError> {
    let dataset = connector.dataset.as_deref().ok_or_else(|| {
        ConnectionError::schema_failed("BigQuery schema discovery requires a dataset")
    })?;

    let tables = table_names(connector, dataset).await?;
    let columns = columns(connector, dataset).await?;
    Ok(build_tree(connector, dataset, tables, columns))
}

async fn table_names(
    connector: &BigQueryConnector,
    dataset: &str,
) -> Result<Vec<String>, ConnectionError> {
    let (project, dataset) = super::dataset::split(dataset, &connector.project);
    let mut names = Vec::new();
    let mut offset = 0;
    loop {
        let sql = table_page_sql(project, dataset, offset);
        let output = connector
            .execute(saya_types::QueryRequest::new(sql, PAGE))
            .await
            .map_err(|_| errors::schema())?;
        let fetched = output.rows.len();
        if output.truncated {
            return Err(ConnectionError::schema_failed(
                "BigQuery table page was truncated; refusing an incomplete schema",
            ));
        }
        append_table_page(output.rows, &mut names)?;
        if fetched < PAGE {
            break;
        }
        offset = offset.checked_add(PAGE).ok_or_else(|| {
            ConnectionError::schema_failed("BigQuery table page offset overflowed")
        })?;
    }
    Ok(names)
}

fn table_page_sql(project: &str, dataset: &str, offset: usize) -> String {
    format!(
        "SELECT table_name FROM `{project}.{dataset}.INFORMATION_SCHEMA.TABLES` \
         ORDER BY table_name LIMIT {PAGE} OFFSET {offset}"
    )
}

fn append_table_page(
    rows: Vec<serde_json::Value>,
    names: &mut Vec<String>,
) -> Result<(), ConnectionError> {
    if names.len().saturating_add(rows.len()) > MAX_SCHEMA_TABLES {
        return Err(ConnectionError::schema_failed(
            "BigQuery dataset has too many tables to enumerate completely; narrow the dataset",
        ));
    }
    names.extend(
        rows.into_iter()
            .map(|row| {
                row.as_array()
                    .and_then(|cells| cells.first())
                    .and_then(|cell| cell.as_str())
                    .map(str::to_owned)
                    .ok_or_else(errors::schema)
            })
            .collect::<Result<Vec<_>, _>>()?,
    );
    Ok(())
}

async fn columns(
    connector: &BigQueryConnector,
    dataset: &str,
) -> Result<Vec<(String, Column)>, ConnectionError> {
    let (project, dataset) = super::dataset::split(dataset, &connector.project);
    let mut rows = Vec::new();
    let mut offset: usize = 0;
    loop {
        let sql = format!(
            "SELECT table_name, column_name, data_type, is_nullable \
             FROM `{project}.{dataset}.INFORMATION_SCHEMA.COLUMNS` \
             ORDER BY table_name, ordinal_position \
             LIMIT {PAGE} OFFSET {offset}",
        );
        let output = connector
            .execute(saya_types::QueryRequest::new(sql, PAGE))
            .await
            .map_err(|_| errors::schema())?;
        let fetched = output.rows.len();
        if fetched == 0 {
            break;
        }
        if rows.len().saturating_add(fetched) > MAX_COLUMNS {
            return Err(ConnectionError::schema_failed(
                "BigQuery dataset is too large to enumerate completely; narrow the dataset",
            ));
        }
        for row in output.rows {
            let cells = row.as_array().ok_or_else(errors::schema)?;
            if cells.len() != 4 {
                return Err(errors::schema());
            }
            let table = cells[0].as_str().ok_or_else(errors::schema)?.to_owned();
            rows.push((
                table,
                Column {
                    name: cells[1].as_str().ok_or_else(errors::schema)?.to_owned(),
                    data_type: cells[2].as_str().ok_or_else(errors::schema)?.to_owned(),
                    nullable: cells[3]
                        .as_str()
                        .unwrap_or("NO")
                        .eq_ignore_ascii_case("YES"),
                },
            ));
        }
        if fetched < PAGE {
            break;
        }
        let Some(next) = offset.checked_add(PAGE) else {
            break;
        };
        offset = next;
    }
    Ok(rows)
}

/// Builds the schema tree: the project is the database, the dataset is the
/// single schema, and every table from TABLES appears — with its columns from
/// COLUMNS attached in declared order, or an empty column list when none were
/// discovered. No foreign keys are synthesized.
fn build_tree(
    connector: &BigQueryConnector,
    dataset: &str,
    tables: Vec<String>,
    columns: Vec<(String, Column)>,
) -> SchemaTree {
    let mut grouped: BTreeMap<String, Vec<Column>> = BTreeMap::new();
    for (table, column) in columns {
        grouped.entry(table).or_default().push(column);
    }
    let built_tables = tables
        .into_iter()
        .map(|name| Table {
            columns: grouped.remove(&name).unwrap_or_default(),
            name,
            primary_key: vec![],
            foreign_keys: vec![],
        })
        .collect();
    let (project, dataset) = super::dataset::split(dataset, &connector.project);
    SchemaTree {
        databases: vec![Database {
            name: project.into(),
            schemas: vec![Schema {
                name: dataset.into(),
                tables: built_tables,
            }],
        }],
    }
}

#[cfg(test)]
#[path = "metadata_tests.rs"]
mod tests;
