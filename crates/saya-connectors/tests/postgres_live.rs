use saya_connectors::{ConnectorOptions, DatabaseConnector, PostgresConnector};
use saya_types::QueryRequest;
use std::str::FromStr;

#[tokio::test]
async fn live_postgres_executes_and_marks_truncation() {
    let Ok(url) = std::env::var("SAYA_TEST_POSTGRES_URL") else {
        return;
    };
    let options = sqlx::postgres::PgConnectOptions::from_str(&url).unwrap();
    let connector = PostgresConnector::from_options(options, ConnectorOptions::default());
    connector.connect().await.unwrap();
    let result = connector
        .execute(QueryRequest::new(
            "SELECT 1 AS id UNION ALL SELECT 2 AS id",
            1,
        ))
        .await
        .unwrap();
    assert_eq!(result.columns, vec!["id"]);
    assert_eq!(result.row_count, 1);
    assert!(result.truncated);
}
