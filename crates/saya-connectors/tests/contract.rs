use async_trait::async_trait;
use saya_connectors::DatabaseConnector;
use saya_types::{ConnectionError, QueryRequest, QueryResult, SchemaTree, SqlDialect};

struct FakeConnector;

#[async_trait]
impl DatabaseConnector for FakeConnector {
    fn dialect(&self) -> SqlDialect {
        SqlDialect::DuckDb
    }
    async fn connect(&self) -> Result<(), ConnectionError> {
        Ok(())
    }
    async fn schema(&self) -> Result<SchemaTree, ConnectionError> {
        Ok(SchemaTree::default())
    }
    async fn execute(&self, request: QueryRequest) -> Result<QueryResult, ConnectionError> {
        Ok(QueryResult::empty(request.sql))
    }
}

#[tokio::test]
async fn a_connector_can_be_used_through_the_public_trait() {
    let connector: Box<dyn DatabaseConnector> = Box::new(FakeConnector);
    connector.connect().await.unwrap();
    let result = connector
        .execute(QueryRequest::new("SELECT 1", 10))
        .await
        .unwrap();
    assert_eq!(result.row_count, 0);
    assert_eq!(connector.dialect(), SqlDialect::DuckDb);
}
