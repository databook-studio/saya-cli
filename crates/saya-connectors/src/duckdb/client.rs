use std::{
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

use async_trait::async_trait;
use duckdb::{AccessMode, Config, Connection, InterruptHandle};
use saya_types::{ConnectionError, QueryRequest, QueryResult, SchemaTree, SqlDialect};

use crate::{ConnectorOptions, DatabaseConnector};

pub struct DuckDbConnector {
    pub(crate) connection: Arc<Mutex<Connection>>,
    pub(crate) interrupt: Arc<InterruptHandle>,
    pub(crate) query_timeout: Duration,
}

impl DuckDbConnector {
    pub async fn open(
        path: &str,
        read_only: bool,
        settings: ConnectorOptions,
    ) -> Result<Self, ConnectionError> {
        let path = path.to_owned();
        let timeout = Duration::from_secs(settings.query_timeout_seconds.max(1));
        tokio::task::spawn_blocking(move || open_sync(&path, read_only, timeout))
            .await
            .map_err(|_| ConnectionError::ConnectionFailed("DuckDB open task failed".into()))?
    }
}

fn open_sync(
    path: &str,
    read_only: bool,
    query_timeout: Duration,
) -> Result<DuckDbConnector, ConnectionError> {
    let mode = if read_only {
        AccessMode::ReadOnly
    } else {
        AccessMode::ReadWrite
    };
    let config = Config::default()
        .access_mode(mode)
        .and_then(|item| item.enable_external_access(false))
        .and_then(|item| item.enable_autoload_extension(false))
        .and_then(|item| item.with("allow_community_extensions", "false"))
        .and_then(|item| item.with("allow_persistent_secrets", "false"))
        .and_then(|item| item.with("lock_configuration", "true"))
        .map_err(|_| {
            ConnectionError::InvalidConfiguration("DuckDB security configuration failed".into())
        })?;
    let connection = Connection::open_with_flags(Path::new(path), config).map_err(|_| {
        ConnectionError::ConnectionFailed("DuckDB database could not be opened".into())
    })?;
    let interrupt = connection.interrupt_handle();
    Ok(DuckDbConnector {
        connection: Arc::new(Mutex::new(connection)),
        interrupt,
        query_timeout,
    })
}

#[async_trait]
impl DatabaseConnector for DuckDbConnector {
    fn dialect(&self) -> SqlDialect {
        SqlDialect::DuckDb
    }
    async fn connect(&self) -> Result<(), ConnectionError> {
        super::execute::ping(self).await
    }
    async fn schema(&self) -> Result<SchemaTree, ConnectionError> {
        super::metadata::schema(self).await
    }
    async fn execute(&self, request: QueryRequest) -> Result<QueryResult, ConnectionError> {
        super::execute::query(self, request).await
    }
    async fn cancel(&self) -> Result<(), ConnectionError> {
        self.interrupt.interrupt();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::open_sync;

    #[test]
    fn engine_blocks_external_access_after_ast_bypass() {
        let path = std::env::temp_dir().join(format!("saya-external-{}.csv", std::process::id()));
        std::fs::write(&path, "id\n1\n").unwrap();
        let connector = open_sync(":memory:", false, Duration::from_secs(1)).unwrap();
        let connection = connector.connection.lock().unwrap();
        assert!(
            connection
                .execute_batch("SET enable_external_access = true")
                .is_err()
        );
        assert!(
            connection
                .execute_batch(&format!(
                    "SELECT * FROM read_csv_auto('{}')",
                    path.display()
                ))
                .is_err()
        );
        let _ = std::fs::remove_file(path);
    }
}
