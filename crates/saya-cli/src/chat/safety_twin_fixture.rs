//! The composed and opened chat run the safety twins drive, and the front each
//! one states its answer through.
//!
//! A twin proves a guarantee of the *live* path: a session's run composed over
//! its folder (`RunComposition::compose`), opened the way the line session and
//! the TUI open it, and driven one turn at a time on the job the session's
//! turns run on. Nothing here asserts on the composition's internals: a twin
//! reads what the run sent the provider, what the model was handed back, what
//! the front was asked, and what the folder holds.
//!
//! [`Fixed`] is the front a twin states its answer through: one answer, given
//! in advance, counted. A front with a person attached (`can_obtain_approval`)
//! answers its own calls; a headless one has nobody to ask and denies at once —
//! the two facts the run's write-shaped advertisement is decided from.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use saya_agent::{AgentEventSink, AgentOutput, ApprovalPolicy, NoopEventSink, SessionGrants};

use super::compose::RunComposition;
use super::compose_fixture::{self, Composed, RootGuard};
use super::{ChatEvent, ChatFront, Question};
use crate::agent::tools::consent::Answer;

/// Load ceiling, not a product bound: how long one scripted turn may take.
pub(super) const RUN: Duration = Duration::from_secs(120);

/// Load ceiling for a background run's own work: an extra pause the test grants
/// a detached command it started through a turn.
pub(super) const WAIT: Duration = Duration::from_secs(20);

/// The workspace-write family's token: the grant that pre-answers a
/// write-shaped call the mode would otherwise ask about.
pub(super) const WORKSPACE_WRITE: &str = saya_agent::WORKSPACE_WRITE_TOKEN;

/// The launch's seed store, holding `tokens` — what `--allow` states.
pub(super) fn grants(tokens: &[&str]) -> SessionGrants {
    let seed = SessionGrants::default();
    for token in tokens {
        seed.grant(token);
    }
    seed
}

/// The session's run, composed and opened over `fixture`, with the folder kept
/// alive: the launch's seeds ride the host the run composes over, exactly as
/// `line_chat::open_run` states them.
pub(super) async fn compose(
    fixture: compose_fixture::Fixture,
    front: Arc<dyn ChatFront>,
    session: &'static str,
    mode: ApprovalPolicy,
    tokens: &[&str],
) -> (RunComposition, RootGuard) {
    let Composed {
        application,
        run,
        root,
    } = fixture.into_parts(front, session, mode);
    application.with_launch_grants(grants(tokens));
    let composition = RunComposition::compose(application, run)
        .await
        .expect("the session's run composes");
    composition.open().await.expect("the run opens");
    (composition, root)
}

/// One turn of the run, waited for under the fixture's load ceiling.
pub(super) async fn turn(composition: &RunComposition, prompt: &str) -> AgentOutput {
    tokio::time::timeout(RUN, composition.job().run_next_turn(&NoopEventSink, prompt))
        .await
        .expect("the turn settles")
        .expect("the turn runs")
}

/// The tool names one request advertises, in order.
pub(super) fn advertised(body: &str) -> Vec<String> {
    let body: serde_json::Value = serde_json::from_str(body).expect("a request body is JSON");
    body["tools"]
        .as_array()
        .expect("a request advertises its tools")
        .iter()
        .filter_map(|tool| tool["function"]["name"].as_str().map(str::to_owned))
        .collect()
}

/// Whether `names` offers `name`.
pub(super) fn offers(names: &[String], name: &str) -> bool {
    names.iter().any(|offered| offered == name)
}

/// The result the model was handed for the call `id`, read off a request body
/// once the call had settled: the loop replays every result as a `tool`
/// message, so a call whose result never came back is absent.
pub(super) fn result_of(bodies: &[String], id: &str) -> serde_json::Value {
    for body in bodies.iter().rev() {
        let body: serde_json::Value = serde_json::from_str(body).expect("a request body is JSON");
        for message in body["messages"].as_array().into_iter().flatten() {
            if message["role"] != "tool" || message["tool_call_id"] != id {
                continue;
            }
            let content = message["content"]
                .as_str()
                .expect("a tool message carries its result as text");
            return serde_json::from_str(content).expect("a tool result is JSON");
        }
    }
    panic!("the model was handed no result for {id}");
}

/// Whether this host's sandbox proves for the run's root, asked of the very
/// probe the root's shell composition asks.
pub(super) fn shell_proves(root: &Path) -> bool {
    crate::agent::tools::shell_policy::prove(root, ApprovalPolicy::Ask).is_some()
}

/// A front that answers every question with one answer stated in advance, and
/// counts what it was put: a person's decision, made before the turn.
pub(super) struct Fixed {
    answer: Answer,
    can_obtain_approval: bool,
    seen: AtomicUsize,
}

impl Fixed {
    /// The front a surface with a person attached puts a call to, answering
    /// `answer`.
    pub(super) fn new(answer: Answer) -> Arc<Self> {
        Self::front(answer, true)
    }

    /// A front with nobody to ask: it denies at once, and the run advertises no
    /// tool whose call it could not ask about.
    pub(super) fn headless() -> Arc<Self> {
        Self::front(Answer::Deny, false)
    }

    fn front(answer: Answer, can_obtain_approval: bool) -> Arc<Self> {
        Arc::new(Self {
            answer,
            can_obtain_approval,
            seen: AtomicUsize::new(0),
        })
    }

    /// How many questions this front has been put so far.
    pub(super) fn seen(&self) -> usize {
        self.seen.load(Ordering::Acquire)
    }
}

#[async_trait]
impl ChatFront for Fixed {
    fn sink(&self) -> Arc<dyn AgentEventSink> {
        Arc::new(NoopEventSink)
    }

    async fn ask(&self, _question: &Question) -> Answer {
        self.seen.fetch_add(1, Ordering::AcqRel);
        self.answer
    }

    fn can_obtain_approval(&self) -> bool {
        self.can_obtain_approval
    }

    fn emit(&self, _event: ChatEvent) {}
}
