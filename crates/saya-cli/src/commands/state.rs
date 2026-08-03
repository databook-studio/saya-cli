use super::output::emit;
use crate::{
    profile_identity::profile_identity,
    render::{RenderFormat, TerminalEvent},
};
use saya_store::{
    AuditEntry, AuditOperation, AuditStatus, AuditStore, SqliteStateStore, StoreError,
};
use saya_types::DatabaseProfile;
use std::time::Duration;

pub(super) fn identity(name: &str, profile: &DatabaseProfile, scope: &std::path::Path) -> String {
    profile_identity(name, profile, scope)
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn audit(
    store: &SqliteStateStore,
    profile_id: &str,
    operation: AuditOperation,
    status: AuditStatus,
    elapsed: Duration,
    rows: Option<usize>,
    truncated: Option<bool>,
    format: RenderFormat,
) {
    if audit_silent(
        store, profile_id, operation, status, elapsed, rows, truncated,
    )
    .await
    .is_err()
    {
        diagnostic(format);
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn audit_silent(
    store: &SqliteStateStore,
    profile_id: &str,
    operation: AuditOperation,
    status: AuditStatus,
    elapsed: Duration,
    rows: Option<usize>,
    truncated: Option<bool>,
) -> Result<(), StoreError> {
    let mut event = AuditEntry::new(profile_id, operation, status, elapsed.as_millis() as u64);
    event.row_count = rows;
    event.truncated = truncated;
    store.record_audit(event).await
}

pub(super) fn diagnostic(format: RenderFormat) {
    emit(
        TerminalEvent::Diagnostic {
            message: "Local state store unavailable; continuing without persistence.".into(),
        },
        format,
    );
}
