//! Unit tests for the agent-capture queue, gap, and evidence constructor
//! (C2/D12): the FIRST match is taken, the bound drops the oldest unmatched
//! outcome (content pairing makes that safe), the queue clears while the
//! gap survives, and the evidence labels what the agent saw.

use super::*;
use crate::agent::tools::AgentCapture;
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
    }
}

#[test]
fn agent_evidence_labels_source_scope_identity_and_cap() {
    let capture = outcome("SELECT id FROM users", "analytics", 3);
    let AgentCaptureOutcome::Captured(capture) = capture else {
        panic!("a captured outcome");
    };
    let evidence = agent_evidence(&capture);
    assert_eq!(evidence.source, EvidenceSource::Agent);
    assert_eq!(
        evidence.scope,
        ResultScope::ModelLimited { row_cap: 100 },
        "the agent saw at most the model row cap"
    );
    assert_eq!(evidence.connection_label, "analytics");
    assert_eq!(
        evidence.connection_identity,
        Some("prof-identity-1".to_string()),
        "the identity is the capture's opaque profile identity"
    );
    assert_eq!(evidence.dialect, SqlDialect::Sqlite);
    assert_eq!(evidence.max_rows, 100, "the cap is the evidence's max_rows");
    assert_eq!(evidence.returned_rows, 3);
    assert_eq!(evidence.started_unix_ms, 1_700_000_000_000);
    assert_eq!(evidence.finished_unix_ms, 1_700_000_000_005);
    assert_eq!(
        evidence.submitted_sql_sha256.len(),
        64,
        "the submitted SQL is named by hash only"
    );
    let line = evidence.human_line();
    assert!(
        line.starts_with("agent: analytics · 3 rows"),
        "the line names source, label, rows: {line}"
    );
    assert!(
        line.contains("model-limited (first 100 rows)"),
        "the line names the scope: {line}"
    );
}

#[test]
fn queue_takes_the_first_matching_and_keeps_the_rest() {
    let mut queue = AgentCaptures::new();
    queue.push(outcome("SELECT 1", "analytics", 1));
    queue.push(refused("SELECT 2", "analytics"));
    queue.push(outcome("SELECT 2", "analytics", 2));
    let taken = queue
        .take_matching("SELECT 2", "analytics")
        .expect("the first matching outcome is taken");
    assert!(
        matches!(taken, AgentCaptureOutcome::Refused { .. }),
        "the FIRST match is taken, even when it is a refusal"
    );
    let taken = queue
        .take_matching("SELECT 1", "analytics")
        .expect("the earlier outcome is untouched");
    let AgentCaptureOutcome::Captured(capture) = taken else {
        panic!("the captured outcome");
    };
    assert_eq!(capture.result.row_count, 1, "the first queued capture");
    // The second SELECT 2 outcome is still queued: only the first matched.
    let taken = queue
        .take_matching("SELECT 2", "analytics")
        .expect("the second match is still queued");
    let AgentCaptureOutcome::Captured(capture) = taken else {
        panic!("the captured outcome");
    };
    assert_eq!(capture.result.row_count, 2);
    assert!(queue.is_empty(), "each match is consumed exactly once");
}

#[test]
fn queue_never_matches_a_different_connection() {
    let mut queue = AgentCaptures::new();
    queue.push(outcome("SELECT 1", "analytics", 1));
    assert!(
        queue.take_matching("SELECT 1", "billing").is_none(),
        "the connection is part of the pairing key"
    );
    assert!(
        queue.take_matching("SELECT 2", "analytics").is_none(),
        "the sql is part of the pairing key"
    );
}

#[test]
fn queue_bound_drops_the_oldest_unmatched() {
    let mut queue = AgentCaptures::new();
    queue.push(outcome("SELECT 0", "c", 0));
    for n in 1..=MAX_UNMATCHED_CAPTURES {
        queue.push(outcome(&format!("SELECT {n}"), "c", n));
    }
    assert!(
        queue.take_matching("SELECT 0", "c").is_none(),
        "beyond the bound the oldest unmatched outcome is dropped"
    );
    assert!(
        queue.take_matching("SELECT 1", "c").is_some(),
        "the survivors are the newest ones"
    );
}

#[test]
fn clear_queue_empties_the_queue_and_keeps_the_gap() {
    let mut queue = AgentCaptures::new();
    queue.push(outcome("SELECT 1", "c", 1));
    queue.gap = Some(CaptureGap::OverBudget);
    queue.clear_queue();
    assert!(queue.is_empty(), "the outcomes are dropped");
    assert_eq!(
        queue.gap,
        Some(CaptureGap::OverBudget),
        "the gap survives: it names the latest promoted query's outcome"
    );
}

#[test]
fn gap_reason_names_the_budget_only_when_refused() {
    assert_eq!(
        CaptureGap::OverBudget.reason(),
        " (larger than 32 MiB)",
        "the refusal's reason is the capture budget"
    );
    assert_eq!(
        CaptureGap::Missing.reason(),
        "",
        "a missing capture invents no reason"
    );
}
