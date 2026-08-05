use super::{
    connection, connection_schema_cache,
    output::{emit, failure},
    state,
};
use crate::{
    config::runtime::RuntimeConfig,
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
    let identity = state::identity(name, profile, &runtime.cache_scope);
    let mut persistence_failed = refresh && store.invalidate_schema(&identity).await.is_err();
    let started = Instant::now();
    let connector = match connection::build(profile, runtime, can_prompt).await {
        Ok(connector) => connector,
        Err(error) if !refresh => {
            return connection_schema_cache::fallback(
                store,
                &identity,
                started,
                error,
                format,
                &mut persistence_failed,
            )
            .await;
        }
        Err(error) => {
            persistence_failed |= state::audit_silent(
                store,
                &identity,
                AuditOperation::SchemaRefresh,
                AuditStatus::Failure,
                started.elapsed(),
                None,
                None,
            )
            .await
            .is_err();
            warn(persistence_failed, format);
            return failure(3, error, format);
        }
    };
    match live_schema(&*connector).await {
        Ok(schema) => {
            persistence_failed |= store.upsert_schema(&identity, &schema).await.is_err();
            persistence_failed |= state::audit_silent(
                store,
                &identity,
                AuditOperation::SchemaRefresh,
                AuditStatus::Success,
                started.elapsed(),
                None,
                None,
            )
            .await
            .is_err();
            warn(persistence_failed, format);
            emit(TerminalEvent::Schema { schema }, format);
            Ok(0)
        }
        Err(error) if !refresh => {
            connection_schema_cache::fallback(
                store,
                &identity,
                started,
                error,
                format,
                &mut persistence_failed,
            )
            .await
        }
        Err(error) => {
            persistence_failed |= state::audit_silent(
                store,
                &identity,
                AuditOperation::SchemaRefresh,
                AuditStatus::Failure,
                started.elapsed(),
                None,
                None,
            )
            .await
            .is_err();
            warn(persistence_failed, format);
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

fn warn(persistence_failed: bool, format: RenderFormat) {
    if persistence_failed {
        state::diagnostic(format);
    }
}
