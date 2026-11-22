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
use crate::agent::tools::shell_policy::ModeCell;
use crate::chat::{
    ChatFront, ChatSession, Ending, JobPort, RunComposition, RunPort, SessionRun, WorkspaceShape,
};
use crate::commands::coding::NativeCodingJob;
use crate::front::SettledTurn;
use crate::local_metrics::LocalMetrics;

/// The run the session is on.
pub(crate) struct Run {
    /// The composition the run was opened from, when the front composed it:
    /// a run the front resumed was handed its job already opened, and its
    /// rollover hands over the job alone.
    composition: Option<RunComposition>,
    /// The chat session driving it, by handle: the line loop sends a line to
    /// it, and a surface whose loop owns the session drives the same one.
    session: Arc<ChatSession>,
    port: Arc<JobPort>,
    job: Arc<NativeCodingJob>,
}

impl Run {
    /// The run and the chat session driving it, over a composition already
    /// opened: the port states it and opens nothing, so the session's prepare
    /// is the composition's one open.
    pub(crate) async fn opened(
        composition: RunComposition,
        front: Arc<dyn ChatFront>,
        metrics: Option<Arc<LocalMetrics>>,
    ) -> Self {
        let job = Arc::clone(composition.job());
        let port = Arc::new(port(&job, metrics));
        let session = Self::chat_session(&port, front).await;
        Self {
            composition: Some(composition),
            session,
            port,
            job,
        }
    }

    /// The run a resume prepared: the engine handed back the job the record
    /// links, at its next epoch, and the front drives it exactly as a composed
    /// one — its turns, its close, and the rollover that may follow.
    pub(super) async fn resumed(
        job: Arc<NativeCodingJob>,
        front: Arc<dyn ChatFront>,
        metrics: Option<Arc<LocalMetrics>>,
    ) -> Self {
        let port = Arc::new(port(&job, metrics));
        let session = Self::chat_session(&port, front).await;
        Self {
            composition: None,
            session,
            port,
            job,
        }
    }

    /// The chat session driving one opened run, stated once for both ways the
    /// front opens one.
    async fn chat_session(port: &Arc<JobPort>, front: Arc<dyn ChatFront>) -> Arc<ChatSession> {
        let session = Arc::new(ChatSession::new(
            Arc::clone(port) as Arc<dyn RunPort>,
            front,
        ));
        session.open().await;
        session
    }

    /// The chat session a turn of this run is sent to.
    pub(crate) fn session(&self) -> &ChatSession {
        &self.session
    }

    /// The chat session a surface outside this module drives: the same session,
    /// by handle, for a surface whose loop owns it rather than sending a line.
    pub(crate) fn chat(&self) -> Arc<ChatSession> {
        Arc::clone(&self.session)
    }

    /// The run's one mode cell, as a surface that states a mode holds it: the
    /// cell every call of this run reads. The composition's cell is the job's
    /// own (`RunComposition` reads it back from the job), and a resumed run's
    /// cell is the one the resume stated the session's mode on — so the job's
    /// cell answers for both, and a mode stated on it reaches the next call.
    pub(crate) fn mode_cell(&self) -> ModeCell {
        self.job.approval_mode()
    }

    /// The port this run's turns run through, for the operations a surface
    /// states without running a turn (the chat layer's `ChatOps` holds it and
    /// never enters it).
    pub(crate) fn run_port(&self) -> Arc<dyn RunPort> {
        Arc::clone(&self.port) as Arc<dyn RunPort>
    }

    /// The run a rollover prepared, over the same port shape the first run had.
    pub(super) fn successor(
        composition: RunComposition,
        session: ChatSession,
        port: Arc<JobPort>,
    ) -> Self {
        Self {
            job: Arc::clone(composition.job()),
            composition: Some(composition),
            session: Arc::new(session),
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

/// What one line handed back: the turn as it settled, the engine's own report of
/// it, and what its boundary said.
pub(crate) struct Sent {
    /// How the turn ended and what its stream reported.
    pub(crate) turn: SettledTurn,
    /// The typed output the turn's engine stated, taken once here: a surface
    /// that reads the turn's own report — the ask's exit-code table, a
    /// candidate attempt's designation — reads it off this, never off a
    /// rendering of it.
    pub(crate) output: Option<saya_agent::AgentOutput>,
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
    drop((session, port));
    match composition {
        Some(composition) => {
            drop(job);
            Ending::new(composition)
        }
        None => Ending::adopted(job),
    }
}
