//! The settle-side gates this packet pins: a turn that ends on a pending
//! clarification never authorises the TUI's automatic compaction — the
//! REPL's identical rule (a pending clarification or a stop is never
//! authority for automatic work, `session_auto_compact`) — and a stream
//! whose worker died without ever posting `Done` settles exactly once as
//! an errored stream, instead of wedging the session busy forever.

use super::*;
use crate::interactive::session::runtime::SessionRuntime;
use saya_agent::ApprovalPolicy;
use std::path::PathBuf;

/// A turn at ~96% of a known window whose completed output carries the
/// typed `ClarificationNeeded` event never starts the automatic
/// compaction: the question pends, and compacting before the user answers
/// would rewrite the conversation under it.
#[test]
fn a_turn_that_ends_on_a_pending_clarification_never_compacts() {
    let (mut app, mut state) = app_with_model("gpt-4o");
    for index in 0..5 {
        state.record_turn(format!("q{index}"), format!("a{index}"), false, Vec::new());
    }
    let clarification = AgentOutput {
        events: vec![AgentEvent::ClarificationNeeded {
            question: "which connection?".into(),
            options: vec!["primary".into()],
        }],
        ..output(TokenUsage::new(122_880, 20))
    };
    let fired = run_turn_maybe_compacting(
        &mut app,
        &mut state,
        vec![
            answering_report(122_880),
            StreamMsg::Done(Ok(clarification)),
        ],
    );
    assert!(
        !fired,
        "a pending clarification is never authority for automatic work"
    );
    assert!(
        app.compact_task.is_none(),
        "no compaction worker may run before the user answers"
    );
}

/// The control: the same turn WITHOUT the clarification event does start
/// the compaction — the gate is the only difference between the two runs.
#[test]
fn the_same_turn_without_a_clarification_still_compacts() {
    let (mut app, mut state) = app_with_model("gpt-4o");
    for index in 0..5 {
        state.record_turn(format!("q{index}"), format!("a{index}"), false, Vec::new());
    }
    let fired = run_turn_maybe_compacting(
        &mut app,
        &mut state,
        vec![
            answering_report(122_880),
            StreamMsg::Done(Ok(output(TokenUsage::new(122_880, 20)))),
        ],
    );
    assert!(fired, "the control fires: the gate is the only difference");
    assert!(app.compact_task.is_some(), "one worker is in flight");
}

/// A stream whose sender was dropped with no `Done` — the spawned worker
/// died — settles exactly once as an errored stream: the dead stream is
/// cleared, the transcript shows the failure, and the session's question
/// is abandoned (no longer Active), so queued prompts can run.
#[test]
fn a_worker_that_dies_without_done_settles_an_errored_stream() {
    let mut h = harness();
    let (mut app, mut state) = (idle_app(), SessionState::new("s1", None, "test-model"));
    let scope = h
        .session
        .begin_question(&h.runtime)
        .expect("the question begins");
    let (tx, rx) = unbounded_channel();
    drop(tx);
    app.request.stream = Some(Stream {
        rx,
        cancel: scope.cancellation(),
        prompt: "question".into(),
        question_scope: Some(scope.clone()),
    });
    assert!(
        app.drain_stream_with_session(&mut state, &mut h.session),
        "the vanished worker's stream settles"
    );
    assert!(app.request.stream.is_none(), "the dead stream is cleared");
    assert!(
        app.transcript
            .blocks()
            .iter()
            .any(|block| block.kind == BlockKind::Error
                && block.text.contains("stopped unexpectedly")),
        "the transcript shows the worker's death"
    );
    let fresh = h
        .session
        .begin_question(&h.runtime)
        .expect("a following begin succeeds after the abandonment");
    assert!(
        !fresh.is_same_question(&scope),
        "the question is no longer Active: the errored turn abandoned it"
    );
}

/// A normal `Done` whose sender then drops settles exactly once: no
/// synthesized "stopped unexpectedly" block rides the same batch.
#[test]
fn a_normal_done_followed_by_a_sender_drop_settles_exactly_once() {
    let (mut app, mut state) = app_with_model("gpt-4o");
    app.request.stream = Some(stream_with(vec![StreamMsg::Done(Ok(output(
        TokenUsage::new(100, 20),
    )))]));
    assert!(app.drain_stream(&mut state), "the turn finished");
    assert!(
        !app.transcript
            .blocks()
            .iter()
            .any(|block| block.text.contains("stopped unexpectedly")),
        "a normal Done never synthesizes a vanished-worker settle"
    );
}
pub(crate) fn app_with_model(model: &str) -> (App, SessionState) {
    (idle_app(), SessionState::new("s1", None, model))
}

pub(crate) fn stream_with(messages: Vec<StreamMsg>) -> Stream {
    let (tx, rx) = unbounded_channel();
    for message in messages {
        let _ = tx.send(message);
    }
    Stream {
        rx,
        cancel: CancellationToken::new(),
        prompt: "question".into(),
        question_scope: None,
    }
}

pub(crate) fn output(usage: TokenUsage) -> AgentOutput {
    AgentOutput {
        answer: "the answer".into(),
        events: Vec::new(),
        used_bounded_sql_query: false,
        tool_metadata: Vec::new(),
        usage,
        learning_usage: None,
        truncated: false,
        answer_sql: None,
    }
}

pub(crate) fn answering_report(input: u64) -> StreamMsg {
    StreamMsg::Event(AgentEvent::usage(
        UsageCall::Answer,
        TokenUsage::new(input, 20),
    ))
}

/// Runs one turn through `drain_stream`, returning whether a compaction
/// worker started. The turn needs enough history for a plan to exist — five
/// recorded turns — so a firing decision actually starts the worker.
pub(crate) fn run_turn_maybe_compacting(
    app: &mut App,
    state: &mut SessionState,
    messages: Vec<StreamMsg>,
) -> bool {
    app.request.stream = Some(stream_with(messages));
    assert!(app.drain_stream(state), "the turn finished");
    app.compact_task.is_some()
}
struct QuestionHarness {
    runtime: crate::config::runtime::RuntimeConfig,
    session: SessionRuntime,
    root: PathBuf,
}

impl Drop for QuestionHarness {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn harness() -> QuestionHarness {
    let root = std::env::temp_dir().join(format!(
        "saya-tui-question-{}-{}",
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
        "tui-question-scope",
        ApprovalPolicy::Ask,
        &root,
    )
    .expect("a real session runtime acquires");
    QuestionHarness {
        runtime,
        session,
        root,
    }
}
