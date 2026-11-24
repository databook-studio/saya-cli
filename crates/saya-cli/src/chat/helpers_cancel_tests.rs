//! The operator's Esc mid-helper: the turn ends and the helper ends with it.
//!
//! The turn's own stop signal is what an attended surface cancels, so it is the
//! token the schedule that runs the helper listens to — the run's token never
//! is: Esc ends one turn of an open run and never the run.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use saya_agent::{AgentEventSink, ApprovalPolicy, NoopEventSink};
use serde_json::json;

use super::compose::RunComposition;
use super::compose_fixture::{self, Composed, RootGuard};
use super::{
    Admission, ChatEvent, ChatFront, ChatOutcome, ChatSession, JobPort, Question, RunPort,
};
use crate::agent::tools::consent::Answer;
use crate::commands::coding::parent_turn_provider::{Reply, Scripted};

/// Load ceiling, not a product bound: how long a scripted turn may take.
const RUN: Duration = Duration::from_secs(120);

/// Load ceiling for the settle: how long a helper given up is awaited. Well
/// under the fixture provider's own five-second request timeout, so a helper
/// that merely timed out could not settle this fast — only the turn's own stop
/// reaching it could.
const SETTLE: Duration = Duration::from_secs(2);

/// The helper the scripted root delegates to.
const HELPER: &str = "helper-1";

/// A front with nobody to ask and nothing to draw: these rows read the session.
#[derive(Default)]
struct QuietFront {
    ended: Mutex<Vec<ChatEvent>>,
}

impl QuietFront {
    fn turn_ends(&self) -> MutexGuard<'_, Vec<ChatEvent>> {
        self.ended.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[async_trait]
impl ChatFront for QuietFront {
    fn sink(&self) -> Arc<dyn AgentEventSink> {
        Arc::new(NoopEventSink)
    }

    async fn ask(&self, _question: &Question) -> Answer {
        Answer::Deny
    }

    fn emit(&self, event: ChatEvent) {
        self.turn_ends().push(event);
    }
}

/// The composed and opened run over one fixture, with the folder kept alive.
async fn compose_case(
    fixture: compose_fixture::Fixture,
    front: Arc<dyn ChatFront>,
    session: &'static str,
) -> (RunComposition, RootGuard) {
    let Composed {
        application,
        run,
        root,
    } = fixture.into_parts(front, session, ApprovalPolicy::Ask);
    let composition = RunComposition::compose(application, run)
        .await
        .expect("the session's run composes");
    composition.open().await.expect("the run opens");
    (composition, root)
}

/// Waits until `count` requests have reached the scripted provider.
async fn arrived(provider: &Scripted, count: usize) {
    let waited = tokio::time::timeout(RUN, async {
        while provider.arrived() < count {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    assert!(
        waited.is_ok(),
        "the scripted provider was reached {} times",
        count
    );
}

/// Esc while a helper is working ends the turn — and the helper with it: no
/// helper is left running once the turn is over.
#[tokio::test]
async fn an_escape_mid_helper_ends_the_turn_and_the_helper() {
    let provider = Scripted::start_with_script(vec![Reply::Call {
        name: "helpers",
        arguments: json!({
            "description": "look around",
            "prompt": "find the tracked file in this folder",
            "subagent_type": "explore",
            "agent_id": HELPER,
        }),
    }]);
    // The helper's own request is the second one, and it is held: the helper is
    // working, and would stay working, until something stops it.
    provider.hold_request(1);
    let fixture = compose_fixture::in_place("helpers-escape", &provider.endpoint).await;
    let front = Arc::new(QuietFront::default());
    let (composition, _root) = compose_case(
        fixture,
        Arc::clone(&front) as Arc<dyn ChatFront>,
        "session-escape",
    )
    .await;
    let job = Arc::clone(composition.job());
    let session = Arc::new(ChatSession::new(
        Arc::new(JobPort::opened(Arc::clone(&job))) as Arc<dyn RunPort>,
        Arc::clone(&front) as Arc<dyn ChatFront>,
    ));
    session.open().await;
    let sending = {
        let session = Arc::clone(&session);
        tokio::spawn(async move {
            session
                .send("use a helper to find where the tracked file is".to_owned())
                .await
        })
    };
    // The root's own answer, then the helper's own request: the helper is
    // working, and nothing but something stopping it would end it.
    arrived(&provider, 2).await;
    assert_eq!(
        job.active_helpers().await,
        1,
        "the helper is working when the operator escapes"
    );

    // The operator's Esc, then the whole path it must settle: the turn ends
    // cancelled, and the schedule that ran the helper drains — while the
    // provider is still holding the helper's answer, which its own request
    // timeout would only release five seconds from now.
    session.cancel().await;
    let ended = tokio::time::timeout(SETTLE, async {
        let admission = sending.await.expect("the sending task runs");
        (admission, job.join().await)
    })
    .await
    .expect("the turn and its helper end on the operator's cancel");
    assert_eq!(ended.0, Admission::Ran, "the message ran as a turn");
    assert!(
        front
            .turn_ends()
            .iter()
            .any(|event| matches!(event, ChatEvent::TurnEnded(outcome) if outcome == &ChatOutcome::Cancelled)),
        "the turn ended cancelled"
    );
    if let Ok(report) = ended.1 {
        assert!(
            matches!(
                report.stop,
                Some(saya_harness::coding::coordinator::ScheduleStop::Cancelled(
                    _
                ))
            ),
            "the helper's schedule stopped on the turn's own stop: {report:?}"
        );
    }
    assert!(
        job.active_helpers().await == 0,
        "no helper is live once the turn ended"
    );
    provider.release();
}
