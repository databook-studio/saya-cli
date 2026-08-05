use super::{
    output::{emit, failure},
    state,
};
use crate::render::{RenderFormat, TerminalEvent};
use saya_store::{AuditOperation, AuditStatus, SchemaStore, SqliteStateStore};
use std::time::Instant;

pub(super) async fn fallback(
    store: &SqliteStateStore,
    identity: &str,
    started: Instant,
    error: saya_types::ConnectionError,
    format: RenderFormat,
    persistence_failed: &mut bool,
) -> Result<i32, Box<dyn std::error::Error>> {
    match store.get_schema(identity).await {
        Ok(Some(cached)) => {
            *persistence_failed |= state::audit_silent(
                store,
                identity,
                AuditOperation::SchemaRefresh,
                AuditStatus::Cached,
                started.elapsed(),
                None,
                None,
            )
            .await
            .is_err();
            warn(*persistence_failed, format);
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
            *persistence_failed = true;
            fail(store, identity, started, error, format, persistence_failed).await
        }
        Ok(None) => fail(store, identity, started, error, format, persistence_failed).await,
    }
}

async fn fail(
    store: &SqliteStateStore,
    identity: &str,
    started: Instant,
    error: saya_types::ConnectionError,
    format: RenderFormat,
    persistence_failed: &mut bool,
) -> Result<i32, Box<dyn std::error::Error>> {
    *persistence_failed |= state::audit_silent(
        store,
        identity,
        AuditOperation::SchemaRefresh,
        AuditStatus::Failure,
        started.elapsed(),
        None,
        None,
    )
    .await
    .is_err();
    warn(*persistence_failed, format);
    failure(3, error, format)
}

fn warn(persistence_failed: bool, format: RenderFormat) {
    if persistence_failed {
        state::diagnostic(format);
    }
}
