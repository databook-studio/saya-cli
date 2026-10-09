//! The TUI stop keys go through the question scope: the first stop captures
//! the application-side stop instant and the real settle path
//! (`drain_stream_with_session` → `settle_done`) records exactly one
//! CancellationToLocalSettlement sample at local settlement; a natural
//! completion records none, and a scope-less stream still cancels its token.

use super::*;
use crate::agent::question_scope::QuestionScope;
use crate::interactive::session::runtime::SessionRuntime;
use crate::interactive::session::state::SessionState;
use crate::interactive::tui::agent::messages::{Stream, StreamMsg};
use crate::interactive::tui::application::tests_support::{idle_app, unused_runtime};
use crate::local_metrics::{LocalMetrics, LocalPhaseSnapshot, phase_index};
use saya_agent::{AgentMetricPhase, ApprovalPolicy, CancellationToken};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc::unbounded_channel;

struct Harness {
    runtime: crate::config::runtime::RuntimeConfig,
    session: SessionRuntime,
    root: PathBuf,
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn harness(tag: &str) -> Harness {
    let root = std::env::temp_dir().join(format!(
        "saya-tui-stop-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let runtime = unused_runtime();
    let session = SessionRuntime::acquire(
        &runtime,
        None,
        false,
        None,
        "tui-stop-metrics",
        ApprovalPolicy::Ask,
        &root,
    )
    .expect("a real session runtime acquires");
    Harness {
        runtime,
        session,
        root,
    }
}

/// A scope whose collector the test holds, the way a real turn's scope
/// carries its local metrics.
fn collector_scope(
    runtime: &crate::config::runtime::RuntimeConfig,
) -> (QuestionScope, Arc<LocalMetrics>) {
    let collector = Arc::new(LocalMetrics::new());
    let scope = QuestionScope::with_metrics(
        runtime,
        CancellationToken::new(),
        Some(Arc::clone(&collector)),
    );
    (scope, collector)
}

/// A stream wired the way production `spawn` wires one: the stop handle is
/// the scope's own token and the scope rides the stream for settlement.
fn scoped_stream(messages: Vec<StreamMsg>, scope: &QuestionScope) -> Stream {
    let (tx, rx) = unbounded_channel();
    for message in messages {
        let _ = tx.send(message);
    }
    Stream {
        rx,
        cancel: scope.cancellation(),
        prompt: "question".into(),
        question_scope: Some(scope.clone()),
    }
}

fn stop_samples(snapshot: &LocalPhaseSnapshot) -> u64 {
    snapshot.phases
        [phase_index(AgentMetricPhase::CancellationToLocalSettlement).expect("known phase indexed")]
    .samples
}

fn output() -> saya_agent::AgentOutput {
    saya_agent::AgentOutput {
        answer: "the answer".into(),
        events: Vec::new(),
        used_bounded_sql_query: false,
        tool_metadata: Vec::new(),
        usage: saya_agent::TokenUsage::new(10, 5),
        learning_usage: None,
        truncated: false,
        answer_sql: None,
    }
}

/// The worker's confirmation of an app-requested stop, the way the real
/// settle path receives one.
fn stop_message() -> StreamMsg {
    StreamMsg::Done(Err("request cancelled".into()))
}

/// Esc on a scoped stream cancels the token and the real settle path records
/// exactly one CancellationToLocalSettlement sample for the turn.
#[test]
fn esc_then_settlement_records_one_stop_sample() {
    let mut h = harness("esc");
    let (mut app, mut state) = (idle_app(), SessionState::new("s1", None, "test-model"));
    let (scope, _collector) = collector_scope(&h.runtime);
    app.request.stream = Some(scoped_stream(vec![stop_message()], &scope));
    handle_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(
        app.request
            .stream
            .as_ref()
            .is_some_and(|stream| stream.cancel.is_cancelled()),
        "Esc cancels the stream's token through the scope"
    );
    assert!(
        app.drain_stream_with_session(&mut state, &mut h.session),
        "the stopped turn finished"
    );
    let taken = h
        .session
        .take_question_metrics()
        .expect("the settled snapshot is kept on the runtime");
    assert_eq!(
        stop_samples(&taken),
        1,
        "exactly one cancellation-to-local-settlement sample"
    );
}

/// A natural completion with no stop press keeps the collector's snapshot
/// but the stop-settlement cell stays empty: no instant, no sample.
#[test]
fn natural_completion_records_no_stop_sample() {
    let mut h = harness("natural");
    let (mut app, mut state) = (idle_app(), SessionState::new("s1", None, "test-model"));
    let (scope, _collector) = collector_scope(&h.runtime);
    app.request.stream = Some(scoped_stream(vec![StreamMsg::Done(Ok(output()))], &scope));
    assert!(
        app.drain_stream_with_session(&mut state, &mut h.session),
        "the turn finished"
    );
    let taken = h
        .session
        .take_question_metrics()
        .expect("the collector's snapshot is kept on a natural completion");
    assert_eq!(
        stop_samples(&taken),
        0,
        "no stop was requested: the cell stays unavailable"
    );
}

/// Two stop presses capture the instant once — the first stop wins — so the
/// settle records one sample, not two.
#[test]
fn two_stops_record_one_sample() {
    let mut h = harness("twice");
    let (mut app, mut state) = (idle_app(), SessionState::new("s1", None, "test-model"));
    let (scope, _collector) = collector_scope(&h.runtime);
    app.request.stream = Some(scoped_stream(vec![stop_message()], &scope));
    handle_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    handle_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(
        app.drain_stream_with_session(&mut state, &mut h.session),
        "the stopped turn finished"
    );
    let taken = h
        .session
        .take_question_metrics()
        .expect("the settled snapshot is kept on the runtime");
    assert_eq!(
        stop_samples(&taken),
        1,
        "the first stop wins and settles once"
    );
}

/// A stream with no scope still stops on Esc through the bare token, says
/// only that the stop was requested, and records nothing.
#[test]
fn scopeless_stream_still_cancels_on_esc() {
    let mut app = idle_app();
    let (_tx, rx) = unbounded_channel();
    let cancel = CancellationToken::new();
    app.request.stream = Some(Stream {
        rx,
        cancel: cancel.clone(),
        prompt: "question".into(),
        question_scope: None,
    });
    handle_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(cancel.is_cancelled(), "the bare token is cancelled");
    let last = app.transcript.blocks().last().expect("the stop posts");
    assert_eq!(
        last.text,
        "Stop requested — waiting for the worker to confirm."
    );
}
