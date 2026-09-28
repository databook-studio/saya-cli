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

/// The snapshot refusal message per gap: the budget for an over-budget
/// refusal, the model-view reason for a truncated or redacted model view
/// (R3 decision 3), and no invented reason for a missing capture.
#[test]
fn the_gap_message_names_each_reason() {
    assert_eq!(
        CaptureGap::OverBudget.message(),
        "The latest query's rows were not captured (larger than 32 MiB). \
         Use /export --refresh to re-run it.",
        "the over-budget refusal keeps its budget wording"
    );
    assert_eq!(
        CaptureGap::ModelViewTruncated.message(),
        "The agent received a truncated version of this result, so it was \
         not captured. Use /export --refresh to re-run it.",
        "a truncated model view names the truncation"
    );
    assert_eq!(
        CaptureGap::ModelViewRedacted.message(),
        "The agent received a redacted version of this result, so it was \
         not captured. Use /export --refresh to re-run it.",
        "a redacted model view names the redaction"
    );
    assert_eq!(
        CaptureGap::Missing.message(),
        "The latest query's rows were not captured. Use /export --refresh to re-run it.",
        "a missing capture invents no reason"
    );
}

// -- the whole lane end to end (R3): tool execution → model shaping → TUI ---
// -- capture pairing. The connector is scripted here because this test must --
// -- reach both the agent tools (pub(crate)) and the private TUI pairing. ---

/// The 70,000-char-cell connector from the verified facts: the serialized
/// result crosses the loop's 65,536-byte message cap, so the model's view is
/// a truncated prefix without the second row.
struct SeventyKCellConnector;

#[async_trait::async_trait]
impl saya_connectors::DatabaseConnector for SeventyKCellConnector {
    fn dialect(&self) -> SqlDialect {
        SqlDialect::DuckDb
    }
    async fn connect(&self) -> Result<(), saya_types::ConnectionError> {
        Ok(())
    }
    async fn schema(&self) -> Result<saya_types::SchemaTree, saya_types::ConnectionError> {
        Ok(saya_types::SchemaTree::default())
    }
    async fn execute(
        &self,
        req: saya_types::QueryRequest,
    ) -> Result<QueryResult, saya_types::ConnectionError> {
        Ok(QueryResult {
            columns: vec!["big".into()],
            rows: vec![
                serde_json::json!(["x".repeat(70_000)]),
                serde_json::json!(["second-row-sentinel"]),
            ],
            row_count: 2,
            truncated: false,
            executed_sql: req.sql,
        })
    }
}

/// A truncated model view is refused from `DatabaseTools` execution through
/// the production hook onto the turn's channel, into `drain_stream`'s
/// pairing: the slot stays empty, the gap records the reason, the completion
/// still promotes the selectable query, and the gap's message names it.
#[tokio::test]
async fn a_truncated_model_view_is_refused_from_tool_execution_to_the_drain_pairing() {
    use crate::agent::tools::DatabaseTools;
    use crate::interactive::session_state::SessionState;
    use crate::interactive::tui::agent::spawn::capture_hook;
    use crate::interactive::tui::agent::{Stream, StreamMsg};
    use crate::interactive::tui::application::tests_support::idle_app;
    use saya_agent::{AgentEvent, CancellationToken, ToolExecutor, shape_tool_result};

    // The production hook, built exactly as the TUI builds it, onto the
    // turn's stream channel.
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let events_tx = tx.clone();
    let hook = capture_hook(tx);
    let budget = saya_agent::AgentLimits::default().context_byte_budget;
    let tools = DatabaseTools::new(Some(Box::new(SeventyKCellConnector)), 100, true)
        .with_capture(Some(hook), budget);

    // The loop's message order: the request, then the tool runs (the hook
    // forwards during it), then the completion promotes.
    let _ = events_tx.send(StreamMsg::Event(AgentEvent::tool_requested(
        "bounded_sql_query",
        serde_json::json!({"sql": "SELECT big", "connection": "primary"}),
        None,
    )));
    let value = tools
        .execute(
            "bounded_sql_query",
            serde_json::json!({"sql": "SELECT big", "connection": "primary"}),
        )
        .await
        .expect("the model still receives its full result");
    let _ = events_tx.send(StreamMsg::Event(AgentEvent::ToolCompleted {
        name: "bounded_sql_query".into(),
        summary: "2 rows".into(),
    }));

    // The shaping step: the loop's own shaping, at the loop's own budget,
    // says the model view was cut — without the second row.
    let shaped = shape_tool_result(&value, budget);
    assert!(
        shaped.truncated,
        "a {}-byte serialization exceeds the {}-byte message cap",
        serde_json::to_string(&value).unwrap().len(),
        saya_agent::tool_message_cap(budget)
    );
    assert!(
        !shaped.text.contains("second-row-sentinel"),
        "the model saw a truncated prefix without the second row"
    );

    // The drain pairing, driven through `drain_stream` — the entry point the
    // loop tick uses.
    let (mut app, mut state) = (idle_app(), SessionState::new("s1", None, "test-model"));
    app.request.stream = Some(Stream {
        rx,
        cancel: CancellationToken::new(),
        prompt: "question".into(),
    });
    app.drain_stream(&mut state);
    assert!(
        app.captured.is_none(),
        "nothing is held: the model saw a cut prefix, so there is no capture"
    );
    assert_eq!(
        app.agent_captures.gap,
        Some(CaptureGap::ModelViewTruncated),
        "the drain pairing records the refusal's reason"
    );
    assert_eq!(
        app.last_query.as_ref().map(|q| q.sql.as_str()),
        Some("SELECT big"),
        "the successful completion still promotes the selectable query"
    );
    assert_eq!(
        app.agent_captures.gap.map(super::CaptureGap::message),
        Some(
            "The agent received a truncated version of this result, so it was \
             not captured. Use /export --refresh to re-run it."
                .to_owned()
        ),
        "the snapshot refusal names the reason"
    );
}
