//! The run a line session is on: the composition, the chat session driving it,
//! the port its turns run through, and the job every one of them names.
//!
//! A rollover replaces this whole object — the successor is composed from the
//! session's own facts, under the next job's name — so the pieces a run keeps
//! are stated here, beside the one hand-over that gives the ended run up.

use std::sync::Arc;

use saya_store::{RedactedSession, RunLink};

use super::boundary::Said;
use super::open::{LineSession, session_run};
use crate::chat::{
    ChatFront, ChatSession, Ending, JobPort, RunComposition, RunPort, SessionRun, WorkspaceShape,
};
use crate::commands::coding::NativeCodingJob;
use crate::front::{LineFront, SettledTurn};
use crate::local_metrics::LocalMetrics;

/// The run the session is on.
pub(super) struct Run {
    composition: RunComposition,
    session: ChatSession,
    port: Arc<JobPort>,
    job: Arc<NativeCodingJob>,
}

impl Run {
    /// The run and the chat session driving it, over a composition already
    /// opened: the port states it and opens nothing, so the session's prepare
    /// is the composition's one open.
    pub(super) async fn opened(
        composition: RunComposition,
        front: &Arc<LineFront>,
        metrics: Option<Arc<LocalMetrics>>,
    ) -> Self {
        let job = Arc::clone(composition.job());
        let port = Arc::new(port(&job, metrics));
        let session = ChatSession::new(
            Arc::clone(&port) as Arc<dyn RunPort>,
            Arc::clone(front) as Arc<dyn ChatFront>,
        );
        session.open().await;
        Self {
            composition,
            session,
            port,
            job,
        }
    }

    /// The chat session a turn of this run is sent to.
    pub(super) fn session(&self) -> &ChatSession {
        &self.session
    }

    /// The run a rollover prepared, over the same port shape the first run had.
    pub(super) fn successor(
        composition: RunComposition,
        session: ChatSession,
        port: Arc<JobPort>,
    ) -> Self {
        Self {
            job: Arc::clone(composition.job()),
            composition,
            session,
            port,
        }
    }

    /// The typed output the last turn of this run reported, taken once by the
    /// surface above: the turn's own report, as the engine stated it.
    pub(super) fn take_output(&self) -> Option<saya_agent::AgentOutput> {
        self.port.take_output()
    }

    /// The job every turn of this run names.
    pub(super) fn job(&self) -> &Arc<NativeCodingJob> {
        &self.job
    }

    /// The link a saved record carries to this run: through the chat layer's
    /// own writer, so there is one shape of link.
    pub(super) fn link(&self) -> Option<RunLink> {
        let mut record = RedactedSession::default();
        crate::chat::after_prepare(&mut record, self.job.job_id(), self.job.epoch());
        record.run
    }

    /// The model the run is on: the window table reads it when the session
    /// declared no window of its own.
    pub(super) fn model(&self) -> String {
        self.job.runtime_config().resolved.ai.model.clone()
    }
}

/// The port a session drives one run's turns through: the run's own counters
/// ride it, so a `--verbose` section keeps counting across a rollover.
fn port(job: &Arc<NativeCodingJob>, metrics: Option<Arc<LocalMetrics>>) -> JobPort {
    match metrics {
        Some(metrics) => JobPort::opened(Arc::clone(job)).with_metrics(metrics),
        None => JobPort::opened(Arc::clone(job)),
    }
}

/// What one line handed back: the turn as it settled, and what its boundary
/// said.
pub(crate) struct Sent {
    /// How the turn ended and what its stream reported.
    pub(crate) turn: SettledTurn,
    /// The compaction's own line, when one fired.
    pub(crate) said: Said,
}

/// The run a compaction rolls into, as the boundary asks for it: the session's
/// own facts, resolved only when a compaction actually rolls, under the next
/// job's name.
pub(super) struct Next {
    name: String,
    front: Arc<dyn ChatFront>,
    parts: LineSession,
    shape: WorkspaceShape,
}

impl Next {
    /// The composer of the run a compaction rolls into.
    pub(super) fn new(
        name: String,
        front: Arc<dyn ChatFront>,
        parts: LineSession,
        shape: WorkspaceShape,
    ) -> Self {
        Self {
            name,
            front,
            parts,
            shape,
        }
    }

    /// The run that follows, composed from the session's own facts.
    pub(super) async fn compose(self) -> Result<SessionRun, String> {
        session_run(&self.parts, self.front, self.name, &self.shape).await
    }
}

/// The run a rollover replaces, handed over: the job object owns the folder's
/// lease, so every handle above it goes before the next job is created, and the
/// successor opens on the lease the run that ended released.
///
/// Only a boundary that summarised calls this, and the session checks it has a
/// run before it asks one to: an absent run here is a session handing over a
/// run it does not hold.
pub(super) fn hand_over(run: Option<Run>) -> Ending {
    let Run {
        composition,
        session,
        port,
        job,
    } = run.expect("the boundary hands over the run the session is on");
    drop((session, port, job));
    Ending::new(composition)
}
