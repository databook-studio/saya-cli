//! The pieces a rollover test drives: a session's run, open over the fixture's
//! folder; the run a context change states next; and the readings a test takes
//! of what the next run actually sent and what the record says now.
//!
//! A rollover is the one place two runs of one session meet, so the fixture
//! keeps the application and the folder both runs open over alive for as long as
//! the test holds it.

use std::sync::Arc;
use std::time::Duration;

use saya_agent::{ApprovalPolicy, NoopEventSink};
use saya_config::MemoryMode;
use saya_store::RedactedSession;
use saya_types::JobId;

use super::compose::RunComposition;
use super::compose_fixture::{self, Composed, RootGuard};
use super::operator_front_fixture;
use super::transcript::Transcript;
use super::{ChatFront, SessionRun, WorkspaceShape};
use crate::commands::coding::parent_turn_provider::Scripted;
use crate::commands::coding::{CodingApplication, NativeCodingJob};
use crate::connection::ConnectionRegistry;

/// Load ceiling, not a product bound: how long one scripted turn may take.
pub(super) const RUN: Duration = Duration::from_secs(120);

/// One session's first run, open over the fixture's folder.
pub(super) struct Open {
    /// The application both runs of the session are composed over.
    pub(super) application: Arc<CodingApplication>,
    /// The run the session is on.
    pub(super) run: RunComposition,
    /// Held for its drop: the folder the runs open over goes when the test does.
    _root: RootGuard,
}

/// Composes and opens the session's run over `fixture`.
pub(super) async fn open(
    fixture: compose_fixture::Fixture,
    front: &operator_front_fixture::Fixture,
    session: &str,
) -> Open {
    let Composed {
        application,
        run,
        root,
    } = fixture.into_parts(front.front(), session, ApprovalPolicy::Ask);
    let composed = RunComposition::compose(Arc::clone(&application), run)
        .await
        .expect("the session's run composes");
    composed.open().await.expect("the session's run opens");
    Open {
        application,
        run: composed,
        _root: root,
    }
}

/// The run a context change states next: the session's next job, over the one
/// application and the one folder. A run's connections are its own — the plugin
/// consumes them — and a model change re-resolves none of them.
pub(super) fn next_run(front: Arc<dyn ChatFront>, session: &str) -> SessionRun {
    SessionRun {
        front,
        session: session.to_owned(),
        mode: ApprovalPolicy::Ask,
        budget: crate::plugins::database::DatabaseBudget::defaults(),
        shape: WorkspaceShape::Writable,
        connections: ConnectionRegistry::new(""),
        allow_query_data: true,
        memory: MemoryMode::Off,
        memory_reachable: false,
        capture: None,
        learning: None,
    }
}

/// A saved record with no run link and no conversation: where a session's
/// conversation starts.
pub(super) fn transcript(id: &str) -> Transcript {
    Transcript::from_record(RedactedSession {
        id: id.to_owned(),
        model: "current-model".to_owned(),
        ..RedactedSession::default()
    })
}

/// One turn of a run, waited for under the fixture's load ceiling, reporting the
/// answer it settled on.
pub(super) async fn turn(job: &Arc<NativeCodingJob>, prompt: &str) -> String {
    let output = tokio::time::timeout(RUN, job.run_next_turn(&NoopEventSink, prompt))
        .await
        .expect("the turn settles")
        .expect("the turn runs");
    output.answer
}

/// One request body's messages, in the order the provider received them.
pub(super) fn messages(body: &str) -> Vec<(String, String)> {
    let body: serde_json::Value = serde_json::from_str(body).expect("a request body is JSON");
    body["messages"]
        .as_array()
        .expect("a request carries its messages")
        .iter()
        .map(|message| {
            (
                message["role"].as_str().unwrap_or_default().to_owned(),
                message["content"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect()
}

/// The request the provider received last.
pub(super) fn last_body(provider: &Scripted) -> String {
    provider
        .bodies()
        .pop()
        .expect("the provider received a request")
}

/// A job id the fixture states.
pub(super) fn job_id(id: &str) -> JobId {
    JobId::parse(id).expect("the fixture's job id parses")
}
