//! Unit tests for the agent-capture queue, gap, and evidence constructor
//! (C2/D12): the FIRST match is taken, the bound drops the oldest unmatched
//! outcome (content pairing makes that safe), the queue clears while the
//! gap survives, and the evidence labels what the agent saw.

use super::*;
use crate::agent::tools::{AgentCapture, CaptureRefusalReason};
use saya_types::{EvidenceSource, QueryResult, ResultScope, SqlDialect};

/// One captured outcome for the query (sql, connection) with `rows` rows.
fn outcome(sql: &str, connection: &str, rows: usize) -> AgentCaptureOutcome {
    AgentCaptureOutcome::Captured(AgentCapture {
        sql: sql.to_string(),
        connection: connection.to_string(),
        profile_identity: Some("prof-identity-1".to_string()),
        dialect: SqlDialect::Sqlite,
        result: QueryResult {
            columns: vec!["id".to_string()],
            rows: (0..rows).map(|i| serde_json::json!([i])).collect(),
            row_count: rows,
            truncated: false,
            executed_sql: sql.to_string(),
        },
        row_cap: 100,
        started_unix_ms: 1_700_000_000_000,
        finished_unix_ms: 1_700_000_000_005,
    })
}

fn refused(sql: &str, connection: &str) -> AgentCaptureOutcome {
    AgentCaptureOutcome::Refused {
        sql: sql.to_string(),
        connection: connection.to_string(),
        reason: CaptureRefusalReason::OverBudget,
    }
}

#[cfg(test)]
#[path = "capture_agent_tests/drain_pairing.rs"]
mod drain_pairing;
#[cfg(test)]
#[path = "capture_agent_tests/queue.rs"]
mod queue;
