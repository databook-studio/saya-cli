use super::{
    output::{emit, failure},
    state,
};
use crate::render::{RenderFormat, TerminalEvent};
use saya_store::{AuditOperation, AuditStatus, SchemaStore, SqliteStateStore};
use std::time::Instant;

pub(super) async fn fallback(
    store: &SqliteStateStore,
    name: &str,
    identity: &str,
    started: Instant,
    error: saya_types::ConnectionError,
    format: RenderFormat,
) -> Result<i32, Box<dyn std::error::Error>> {
    match store.get_schema(identity).await {
        Ok(Some(cached)) => {
            state::audit(
                store,
                name,
                AuditOperation::SchemaRefresh,
                AuditStatus::Cached,
                started.elapsed(),
                None,
                None,
                format,
            )
            .await;
            emit(
                TerminalEvent::Diagnostic {
                    message: "Using cached schema metadata; it may be stale.".into(),
                },
                format,
            );
            emit(
                TerminalEvent::Schema {
                    schema: cached.schema,
                },
                format,
            );
            Ok(0)
        }
        Err(_) => {
            state::diagnostic(format);
            fail(store, name, started, error, format).await
        }
        Ok(None) => fail(store, name, started, error, format).await,
    }
}
async fn fail(
    store: &SqliteStateStore,
    name: &str,
    started: Instant,
    error: saya_types::ConnectionError,
    format: RenderFormat,
) -> Result<i32, Box<dyn std::error::Error>> {
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
