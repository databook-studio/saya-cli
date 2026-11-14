//! The TUI answering turn's question lifecycle, proved through the real
//! stream-finalize function (`drain_stream_with_session` → `settle_done`):
//! the scope `begin_question` handed out settles by identity — a
//! clarification reply re-begins the SAME scope on the SAME token, a natural
//! answer or an ordinary error leaves Idle so the next submission is a
//! FRESH scope — and the spawned stream's stop handle is the question's own
//! token, never a minted one.

use super::*;
use crate::agent::question_scope::QuestionScope;
use crate::interactive::session::runtime::SessionRuntime;
use crate::interactive::tui::agent;
use saya_agent::ApprovalPolicy;
use std::path::PathBuf;
use std::time::Duration;

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

fn output(events: Vec<AgentEvent>) -> AgentOutput {
    AgentOutput {
        answer: "the answer".into(),
        events,
        used_bounded_sql_query: false,
        tool_metadata: Vec::new(),
        usage: TokenUsage::new(10, 5),
        learning_usage: None,
        truncated: false,
        answer_sql: None,
    }
}

fn clarification_output() -> AgentOutput {
    output(vec![AgentEvent::ClarificationNeeded {
        question: "which connection?".into(),
        options: vec!["primary".into(), "replica".into()],
    }])
}

#[test]
fn a_clarification_reply_reuses_its_question_scope() {
    let mut h = harness();
    let (mut app, mut state) = (idle_app(), SessionState::new("s1", None, "test-model"));

    // Turn 1 — the completed output carries the typed ClarificationNeeded
    // event: the question pends, and the reply re-begins the SAME scope.
    let first = h
        .session
        .begin_question(&h.runtime)
        .expect("the first question begins");
    let first_token = first.cancellation();
    app.request.stream = Some(scoped_stream(
        vec![StreamMsg::Done(Ok(clarification_output()))],
        &first,
    ));
    assert!(
        app.drain_stream_with_session(&mut state, &mut h.session),
        "the turn finished"
    );
    let reply = h
        .session
        .begin_question(&h.runtime)
        .expect("the reply begins while the question pends");
    assert!(
        reply.is_same_question(&first),
        "a clarification reply re-begins the SAME question scope"
    );
    assert!(
        !reply.cancellation().is_cancelled() && !first_token.is_cancelled(),
        "the reply runs on the first turn's still-live token, not a minted one"
    );

    // Turn 2 — the reply answers naturally (no clarification in the
    // completed output): the question settles Idle and an unrelated new
    // submission mints a FRESH scope.
    app.request.stream = Some(scoped_stream(
        vec![StreamMsg::Done(Ok(output(Vec::new())))],
        &reply,
    ));
    assert!(
        app.drain_stream_with_session(&mut state, &mut h.session),
        "the reply finished"
    );
    let next = h
        .session
        .begin_question(&h.runtime)
        .expect("the next question begins after a natural answer");
    assert!(
        !next.is_same_question(&first),
        "a natural answer settles Idle: the next submission is a fresh question"
    );

    // A transient ClarificationNeeded event is never authority: only the
    // completed output's typed event decides, so this turn settles Idle too.
    app.request.stream = Some(scoped_stream(
        vec![
            StreamMsg::Event(AgentEvent::ClarificationNeeded {
                question: "transient".into(),
                options: Vec::new(),
            }),
            StreamMsg::Done(Ok(output(Vec::new()))),
        ],
        &next,
    ));
    assert!(
        app.drain_stream_with_session(&mut state, &mut h.session),
        "the turn finished"
    );
    let after_transient = h
        .session
        .begin_question(&h.runtime)
        .expect("begins after the transient-only turn");
    assert!(
        !after_transient.is_same_question(&next),
        "a transient event cannot pend the question: it settled Idle"
    );
    // Close that question with a natural end so the error leg can begin.
    app.request.stream = Some(scoped_stream(
        vec![StreamMsg::Done(Ok(output(Vec::new())))],
        &after_transient,
    ));
    assert!(
        app.drain_stream_with_session(&mut state, &mut h.session),
        "the closing turn finished"
    );

    // An ordinary error through the REAL spawn path leaves the session
    // Idle: the question's token is cancelled before the worker starts (no
    // provider is contacted), the spawned stream's stop handle reads that
    // same cancellation — the stream's cancel IS the question's token, not
    // a minted one — and the failed turn abandons, so a following begin
    // succeeds with a fresh scope.
    let failing = h
        .session
        .begin_question(&h.runtime)
        .expect("the failing turn's question begins");
    failing.cancellation().cancel();
    app.request.stream = Some(agent::start(agent::StreamRequest {
        runtime: std::sync::Arc::new(unused_runtime()),
        prompt: "a prompt".into(),
        approval: saya_agent::ApprovalPolicy::ReadOnly,
        policy: saya_agent::SessionPolicy::new(saya_agent::ApprovalPolicy::ReadOnly),
        overrides: crate::agent::runtime::PromptOverrides::default(),
        history: Vec::new(),
        state_db: app.state_db.clone(),
        last_sql: None,
        session: std::sync::Arc::clone(&app.session),
        journal: None,
        agent_mode: saya_agent::AgentMode::Build,
        prior_tool_outcomes: None,
        continuation: None,
        compaction_summary: None,
        question_scope: Some(failing.clone()),
    }));
    assert!(
        app.request
            .stream
            .as_ref()
            .is_some_and(|stream| stream.cancel.is_cancelled()),
        "the spawned stream's stop handle is the question's own token"
    );
    let mut finished = false;
    for _ in 0..400 {
        if app.drain_stream_with_session(&mut state, &mut h.session) {
            finished = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(finished, "the failed turn's stream settled");
    let fresh = h
        .session
        .begin_question(&h.runtime)
        .expect("a following begin succeeds after the error");
    assert!(
        !fresh.is_same_question(&failing),
        "the failed turn abandoned its question: the next begin is a fresh scope"
    );
}
