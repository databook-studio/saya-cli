use super::output::emit;
use crate::{
    profile_identity::profile_identity,
    render::{RenderFormat, TerminalEvent},
};
use saya_store::{
    AuditEntry, AuditOperation, AuditStatus, AuditStore, SqliteStateStore, StoreError,
};
use std::time::Duration;

pub(super) fn identity(profile: &str) -> String {
    profile_identity(profile)
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn audit(
    store: &SqliteStateStore,
    profile: &str,
    operation: AuditOperation,
    status: AuditStatus,
    elapsed: Duration,
    rows: Option<usize>,
    truncated: Option<bool>,
    format: RenderFormat,
) {
    let mut event = AuditEntry::new(
        identity(profile),
        operation,
        status,
        elapsed.as_millis() as u64,
    );
    event.row_count = rows;
    event.truncated = truncated;
    if store.record_audit(event).await.is_err() {
        diagnostic(format);
    }
}

pub(super) async fn ignore(result: Result<(), StoreError>, format: RenderFormat) {
    if result.is_err() {
        diagnostic(format);
    }
}

pub(super) fn diagnostic(format: RenderFormat) {
    emit(
        TerminalEvent::Diagnostic {
            message: "Local state store unavailable; continuing without persistence.".into(),
        },
        format,
    );
}
