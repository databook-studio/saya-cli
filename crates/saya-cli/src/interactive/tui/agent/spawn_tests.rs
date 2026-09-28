//! The spawn-built capture hook forwards capture events onto the turn's
//! stream channel — the `StreamMsg` variants the drain will consume (C2).

use super::*;
use crate::agent::tools::{AgentCapture, CaptureEvent, CaptureRefusalReason};
use saya_types::{QueryResult, SqlDialect};
use tokio::sync::mpsc::unbounded_channel;

#[test]
fn capture_hook_forwards_onto_the_stream_channel() {
    let (tx, mut rx) = unbounded_channel();
    let hook = capture_hook(tx);
    hook(CaptureEvent::Refused {
        sql: "SELECT 1".into(),
        connection: "primary".into(),
        reason: CaptureRefusalReason::OverBudget,
    });
    hook(CaptureEvent::Captured(AgentCapture {
        sql: "SELECT 1".into(),
        connection: "primary".into(),
        profile_identity: None,
        dialect: SqlDialect::Sqlite,
        result: QueryResult::empty("SELECT 1"),
        row_cap: 50,
        started_unix_ms: 1,
        finished_unix_ms: 2,
    }));
    assert!(matches!(
        rx.blocking_recv(),
        Some(StreamMsg::QueryCaptureRefused {
            sql,
            connection,
            reason: CaptureRefusalReason::OverBudget,
        }) if sql == "SELECT 1" && connection == "primary"
    ));
    assert!(matches!(
        rx.blocking_recv(),
        Some(StreamMsg::QueryCaptured(_))
    ));
}
