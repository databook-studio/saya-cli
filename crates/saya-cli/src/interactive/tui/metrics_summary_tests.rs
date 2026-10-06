//! The TUI keeps the last settled question's fixed local-metrics snapshot
//! on the session runtime and hands it to the print-after-restore seam:
//! settlement (through the real `drain_stream_with_session` → `settle_done`
//! path) remembers the collector's snapshot on every outcome, only the last
//! completed question is held, a question without a collector keeps
//! nothing, and the real spawn path wraps the collector for recording.

use super::agent;
use super::agent::approval::StreamRequest;
use super::agent::messages::{Stream, StreamMsg};
use super::application::tests_support::{idle_app, unused_runtime};
use crate::agent::question_scope::QuestionScope;
use crate::interactive::session_runtime::SessionRuntime;
use crate::interactive::session_state::SessionState;
use crate::local_metrics::{LocalMetrics, LocalPhaseSnapshot, phase_index};
use saya_agent::{
    AgentMetric, AgentMetricOutcome, AgentMetricPhase, ApprovalPolicy, CancellationToken,
};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
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
        "saya-tui-metrics-{tag}-{}-{}",
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
        "tui-metrics-summary",
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

/// One recorded establishment phase, the way the sink wrap would feed the
/// collector during a real turn.
fn establishment_sample(collector: &LocalMetrics) {
    collector.record(&AgentMetric::Phase {
        phase: AgentMetricPhase::ProviderEstablishment,
        elapsed_ns: 1_234,
        outcome: AgentMetricOutcome::Succeeded,
    });
}

fn collector_scope(
    runtime: &crate::config::runtime::RuntimeConfig,
    collector: Option<Arc<LocalMetrics>>,
) -> QuestionScope {
    QuestionScope::with_metrics(runtime, CancellationToken::new(), collector)
}

fn establishment_samples(snapshot: &LocalPhaseSnapshot) -> u64 {
    snapshot.phases
        [phase_index(AgentMetricPhase::ProviderEstablishment).expect("known phase indexed")]
    .samples
}

fn settled_stream(messages: Vec<StreamMsg>, scope: &QuestionScope) -> Stream {
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

#[test]
fn settlement_keeps_the_last_collector_snapshot_and_none_without_one() {
    let mut h = harness("settlement");
    let (mut app, mut state) = (idle_app(), SessionState::new("s1", None, "test-model"));

    // Question 1 — a collector pre-fed one establishment sample rides the
    // stream's scope through the REAL drain → settle_done path; settlement
    // remembers its snapshot on the runtime. The next question replaces it:
    // two fresh samples survive, not one — only the last question is held.
    let collector = Arc::new(LocalMetrics::new());
    establishment_sample(&collector);
    let first = collector_scope(&h.runtime, Some(Arc::clone(&collector)));
    app.request.stream = Some(settled_stream(vec![StreamMsg::Done(Ok(output()))], &first));
    assert!(
        app.drain_stream_with_session(&mut state, &mut h.session),
        "the first turn finished"
    );
    let collector = Arc::new(LocalMetrics::new());
    establishment_sample(&collector);
    establishment_sample(&collector);
    let second = collector_scope(&h.runtime, Some(collector));
    app.request.stream = Some(settled_stream(vec![StreamMsg::Done(Ok(output()))], &second));
    assert!(
        app.drain_stream_with_session(&mut state, &mut h.session),
        "the second turn finished"
    );
    let taken = h
        .session
        .take_question_metrics()
        .expect("the settled question's snapshot is kept on the runtime");
    assert_eq!(
        establishment_samples(&taken),
        2,
        "the LAST completed question's snapshot is kept, replacing the earlier one"
    );

    // Question 3 — a scope without a collector records nothing: take stays
    // None, and a later take confirms the seam drains the snapshot once.
    let bare = collector_scope(&h.runtime, None);
    app.request.stream = Some(settled_stream(vec![StreamMsg::Done(Ok(output()))], &bare));
    assert!(
        app.drain_stream_with_session(&mut state, &mut h.session),
        "the third turn finished"
    );
    assert!(
        h.session.take_question_metrics().is_none(),
        "a question without a collector leaves no snapshot behind"
    );
}

#[test]
fn the_spawn_path_wraps_the_collector_and_settlement_keeps_its_snapshot() {
    let mut h = harness("spawn");
    let (mut app, mut state) = (idle_app(), SessionState::new("s1", None, "test-model"));

    // The REAL spawn path (`agent::start`), the shape the question
    // lifecycle test's spawn leg uses: a collector-bearing scope whose
    // token is cancelled before the worker starts, so no provider is
    // contacted and the turn fails fast. The spawn wires the collector
    // into the sink wrap; settlement keeps the collector's snapshot even
    // on this error outcome.
    let collector = Arc::new(LocalMetrics::new());
    establishment_sample(&collector);
    let scope = collector_scope(&h.runtime, Some(collector));
    scope.cancellation().cancel();
    app.request.stream = Some(agent::start(StreamRequest {
        runtime: Arc::new(unused_runtime()),
        prompt: "a prompt".into(),
        approval: ApprovalPolicy::ReadOnly,
        policy: saya_agent::SessionPolicy::new(ApprovalPolicy::ReadOnly),
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
        question_scope: Some(scope.clone()),
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
    let taken = h
        .session
        .take_question_metrics()
        .expect("settlement keeps the collector's snapshot on the error outcome too");
    assert_eq!(
        establishment_samples(&taken),
        1,
        "the snapshot is the scope's own collector (its pre-fed sample), \
         recorded through the wrapped spawn sink without a provider"
    );
}
