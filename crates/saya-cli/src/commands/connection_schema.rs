use super::{
    connection, connection_schema_cache,
    output::{emit, failure},
    state,
};
use crate::{
    config_runtime::RuntimeConfig,
    render::{RenderFormat, TerminalEvent},
};
use saya_connectors::DatabaseConnector;
use saya_store::{AuditOperation, AuditStatus, SchemaStore, SqliteStateStore};
use std::time::Instant;

pub(crate) async fn run(
    name: &str,
    refresh: bool,
    runtime: &RuntimeConfig,
    format: RenderFormat,
    can_prompt: bool,
    store: &SqliteStateStore,
) -> Result<i32, Box<dyn std::error::Error>> {
    let profile = match runtime.named_profile(name) {
        Ok(profile) => profile,
        Err(error) => {
            return failure(
                3,
                saya_types::ConnectionError::InvalidConfiguration(error.to_string()),
                format,
            );
        }
    };
    let identity = state::identity(name);
    if refresh {
        state::ignore(store.invalidate_schema(&identity).await, format).await;
    }
    let started = Instant::now();
    let connector = match connection::build(profile, runtime, can_prompt).await {
        Ok(connector) => connector,
        Err(error) if !refresh => {
            return connection_schema_cache::fallback(
                store, name, &identity, started, error, format,
            )
            .await;
        }
        Err(error) => {
            state::audit(
                store,
                name,
                AuditOperation::SchemaRefresh,
                AuditStatus::Failure,
                started.elapsed(),
                None,
                None,
                format,
            )
            .await;
            return failure(3, error, format);
        }
    };
    match live_schema(&*connector).await {
        Ok(schema) => {
            state::ignore(store.upsert_schema(&identity, &schema).await, format).await;
            state::audit(
                store,
                name,
                AuditOperation::SchemaRefresh,
                AuditStatus::Success,
                started.elapsed(),
                None,
                None,
                format,
            )
            .await;
            emit(TerminalEvent::Schema { schema }, format);
            Ok(0)
        }
        Err(error) if !refresh => {
            connection_schema_cache::fallback(store, name, &identity, started, error, format).await
        }
        Err(error) => {
            state::audit(
                store,
                name,
                AuditOperation::SchemaRefresh,
                AuditStatus::Failure,
                started.elapsed(),
                None,
                None,
                format,
            )
            .await;
            failure(3, error, format)
        }
    }
}
async fn live_schema(
    connector: &dyn DatabaseConnector,
) -> Result<saya_types::SchemaTree, saya_types::ConnectionError> {
    connector.connect().await?;
    connector.schema().await
}
