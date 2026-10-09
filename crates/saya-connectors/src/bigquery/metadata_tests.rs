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

#[test]
fn table_pages_always_carry_offset_for_complete_enumeration() {
    let sql = table_page_sql("project", "analytics", PAGE);
    assert!(sql.contains("LIMIT 5000 OFFSET 5000"));
}

#[test]
fn table_page_refuses_when_the_shared_snapshot_bound_is_exceeded() {
    let rows = (0..=MAX_SCHEMA_TABLES)
        .map(|index| serde_json::json!([format!("table_{index}")]))
        .collect();
    let error = append_table_page(rows, &mut Vec::new()).unwrap_err();
    assert!(error.to_string().contains("too many tables"));
}
