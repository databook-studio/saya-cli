use std::collections::BTreeMap;

use saya_types::{Column, ConnectionError, Database, Schema, SchemaTree, Table};

use super::BigQueryConnector;
use super::errors;
use crate::DatabaseConnector;

/// Upper bound on the column rows fetched in one pass. A real dataset stays
/// well under this; the cap fails closed instead of growing without limit.
const PAGE: usize = 5_000;
const MAX_COLUMNS: usize = 200_000;

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
    let sql = format!(
        "SELECT table_name FROM `{project}.{dataset}.INFORMATION_SCHEMA.TABLES` \
         ORDER BY table_name LIMIT {PAGE}",
    );
    let output = connector
        .execute(saya_types::QueryRequest::new(sql, PAGE))
        .await
        .map_err(|_| errors::schema())?;
    output
        .rows
        .into_iter()
        .map(|row| {
            row.as_array()
                .and_then(|cells| cells.first())
                .and_then(|cell| cell.as_str())
                .map(str::to_owned)
                .ok_or_else(errors::schema)
        })
        .collect()
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
mod tests {
    use super::*;
    use crate::ConnectorOptions;
    use rsa::pkcs8::EncodePrivateKey;

    fn connector(project: &str) -> BigQueryConnector {
        let key = rsa::RsaPrivateKey::new(&mut rsa::rand_core::OsRng, 2048).unwrap();
        let private_key = key
            .to_pkcs8_pem(rsa::pkcs8::LineEnding::LF)
            .unwrap()
            .to_string();
        let json = format!(
            r#"{{"client_email":"r@p.iam.gserviceaccount.com","private_key":{private_key:?},"token_uri":"https://oauth2.googleapis.com/token"}}"#
        );
        BigQueryConnector::new(
            project.into(),
            Some("analytics".into()),
            None,
            None,
            json,
            ConnectorOptions::default(),
        )
        .unwrap()
    }

    #[test]
    fn build_tree_groups_columns_by_table_and_attaches_empty_for_columnless_tables() {
        let connector = connector("my-proj");
        let tables = vec!["orders".into(), "empty_view".into()];
        let columns = vec![
            (
                "orders".into(),
                Column {
                    name: "id".into(),
                    data_type: "INT64".into(),
                    nullable: false,
                },
            ),
            (
                "orders".into(),
                Column {
                    name: "amount".into(),
                    data_type: "FLOAT64".into(),
                    nullable: true,
                },
            ),
        ];
        let tree = build_tree(&connector, "analytics", tables, columns);
        assert_eq!(tree.databases.len(), 1);
        assert_eq!(tree.databases[0].name, "my-proj");
        assert_eq!(tree.databases[0].schemas.len(), 1);
        assert_eq!(tree.databases[0].schemas[0].name, "analytics");
        let tables = &tree.databases[0].schemas[0].tables;
        assert_eq!(tables.len(), 2);
        let orders = tables.iter().find(|t| t.name == "orders").unwrap();
        assert_eq!(orders.columns.len(), 2);
        assert!(!orders.columns[0].nullable);
        assert!(orders.columns[1].nullable);
        let empty = tables.iter().find(|t| t.name == "empty_view").unwrap();
        assert!(empty.columns.is_empty());
    }

    #[test]
    fn qualified_dataset_is_named_by_its_owning_project() {
        // The agent writes SQL from these names. Reporting the billing project
        // as the database would send it to `my-proj.bigquery-public-data...`,
        // which resolves to nothing.
        let connector = connector("my-proj");
        let tree = build_tree(
            &connector,
            "bigquery-public-data.usa_names",
            vec!["usa_1910_2013".into()],
            vec![],
        );
        assert_eq!(tree.databases[0].name, "bigquery-public-data");
        assert_eq!(tree.databases[0].schemas[0].name, "usa_names");
    }

    #[test]
    fn build_tree_reports_no_foreign_keys_or_primary_keys() {
        let connector = connector("p");
        let tree = build_tree(
            &connector,
            "d",
            vec!["t".into()],
            vec![(
                "t".into(),
                Column {
                    name: "c".into(),
                    data_type: "INT64".into(),
                    nullable: false,
                },
            )],
        );
        let table = &tree.databases[0].schemas[0].tables[0];
        assert!(table.foreign_keys.is_empty());
        assert!(table.primary_key.is_empty());
    }

    #[test]
    fn schema_discovery_sql_parses_as_a_read_only_bigquery_statement() {
        // The schema SQL is connector-generated and still goes through the
        // safety layer, so it must parse under the BigQuery dialect and be
        // accepted as a read. This guards the backtick-qualified INFORMATION
        // SCHEMA path against a parser change that would silently break
        // discovery.
        let sql = format!(
            "SELECT table_name, column_name, data_type, is_nullable \
             FROM `{project}.analytics.INFORMATION_SCHEMA.COLUMNS` \
             ORDER BY table_name, ordinal_position LIMIT 5000",
            project = "my-proj",
        );
        assert!(
            crate::prepare_bigquery_sql(&sql, 5000).is_ok(),
            "schema SQL must be accepted"
        );
        let tables_sql = format!(
            "SELECT table_name FROM `{project}.analytics.INFORMATION_SCHEMA.TABLES` \
             ORDER BY table_name LIMIT 5000",
            project = "my-proj",
        );
        assert!(crate::prepare_bigquery_sql(&tables_sql, 5000).is_ok());
    }
}
