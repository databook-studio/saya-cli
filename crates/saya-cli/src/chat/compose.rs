//! Composing one chat session's run.
//!
//! A session is a job run with a front end: what it hands the run core at open
//! is a job request, the run's per-turn inputs, the run's one mode cell, and the
//! operator every call of the run is put to. [`RunComposition`] builds those
//! from what the session resolved — its databases, the shape of its folder, its
//! mode, its privacy and memory settings — and opens the run with them, so the
//! session above it knows no epoch, no slot, no lease.
//!
//! **Nothing is asked to open a run.** Consent to open is the session's mode:
//! the composition states an approval for exactly the request it just built, so
//! composing and opening are silent — a person is asked about a *call* of the
//! run, never about the run itself.
//!
//! **The mode stays switchable.** The run is composed as an attached launch, so
//! the cell [`RunComposition::mode_cell`] hands out is the same cell the root's
//! `shell` and the database lane read on every call: a mode switched between two
//! calls decides the second.
//!
//! **The database is the plugin's.** With a database resolved the registry holds
//! the database plugin's chat surface, which states the database section of the
//! run's prompt, offers the SQL tools, and — through its lane — decides them;
//! with none the same registry is composed and the plugin simply does not
//! activate, so the run carries zero database bytes.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use saya_agent::{ApprovalPolicy, CancellationToken, NoopEventSink};
use saya_config::MemoryMode;
use saya_types::JobId;

use super::compose_plugins::{self, PluginSetup};
use super::workspace_shape::WorkspaceShape;
use super::{ChatFront, SessionOperator};
use crate::agent::tools::CaptureHook;
use crate::agent::tools::consent::Operator;
use crate::agent::tools::shell_policy::ModeCell;
use crate::commands::coding::{
    CodingApplication, CodingAuthorization, CodingError, CodingJobRequest, Launch, NativeCodingJob,
    TurnInputs,
};
use crate::connection::ConnectionRegistry;

/// A session's run, as the session states it: the facts the composition builds
/// the run's request, inputs, mode cell and operator from.
pub(crate) struct SessionRun {
    /// The surface the run's questions are put to.
    pub(crate) front: Arc<dyn ChatFront>,
    /// The session's name: the run's job id and the label its request states.
    pub(crate) session: String,
    /// The session's approval mode — the consent to open, and the mode the
    /// run's one cell starts in.
    pub(crate) mode: ApprovalPolicy,
    /// What a run may do with the folder the session opened over.
    pub(crate) shape: WorkspaceShape,
    /// The databases the session resolved, primary first: the plugin's
    /// connections, and what its prompt section is composed from.
    pub(crate) connections: ConnectionRegistry,
    /// Whether query rows may reach the model.
    pub(crate) allow_query_data: bool,
    /// The session's memory mode, and whether it can honour it.
    pub(crate) memory: MemoryMode,
    pub(crate) memory_reachable: bool,
    /// The session's query-capture hook, when the front previews what it runs.
    pub(crate) capture: Option<CaptureHook>,
}

/// One session's composed run: the job that runs it, what its one open is given,
/// the operator its calls are put to, and the mode cell every call reads.
pub(crate) struct RunComposition {
    job: Arc<NativeCodingJob>,
    /// What the run is opened with, taken by the one open — so a second open
    /// meets the job's own `Busy` rather than a second turn.
    inputs: Mutex<Option<TurnInputs>>,
    /// The session's operator, as it was attached to the run.
    operator: Arc<SessionOperator>,
    /// The run's one mode cell.
    mode: ModeCell,
}

impl RunComposition {
    /// Builds the session's run over `application` — the request, the run's
    /// plugin registry, the mode cell and the operator — and creates the job.
    /// Nobody is asked: the approval the job is created under is the one this
    /// composition just stated for its own request.
    pub(crate) async fn compose(
        application: Arc<CodingApplication>,
        run: SessionRun,
    ) -> Result<Self, CodingError> {
        let SessionRun {
            front,
            session,
            mode,
            shape,
            connections,
            allow_query_data,
            memory,
            memory_reachable,
            capture,
        } = run;
        let operator = Arc::new(SessionOperator::new(front));
        let application = Launch::Attached(mode).compose(application);
        let application = application.with_operator(Arc::clone(&operator) as Arc<dyn Operator>);
        let request = request(&session, &shape)?;
        // The session's mode is the consent to open: the approval is scoped to
        // the exact request built here, so no question is put to anybody.
        let proposal = application.propose(&request).await?;
        let approval = CodingAuthorization::scoped(&proposal)?;
        let job = application
            .create(request, &approval, CancellationToken::new())
            .await?;
        let plugins = compose_plugins::run_plugins(
            &application,
            &job,
            PluginSetup {
                connections,
                allow_query_data,
                memory,
                memory_reachable,
                capture,
            },
        )?;
        let mode = job.approval_mode();
        Ok(Self {
            job,
            inputs: Mutex::new(Some(TurnInputs {
                policy: None,
                plugins: Some(plugins),
                seed: Vec::new(),
            })),
            operator,
            mode,
        })
    }

    /// Opens the run and runs no turn. After it the run holds its provider,
    /// toolset, plugin sections and lease, and the session's first message is
    /// the run's first turn. Nothing is asked: the run was consented to when it
    /// was composed, under the session's mode.
    pub(crate) async fn open(&self) -> Result<(), CodingError> {
        let inputs = self.inputs().take().unwrap_or_default();
        self.job.run_prepare(&NoopEventSink, inputs).await
    }

    /// The run's job: what the session's turns are run on.
    pub(crate) fn job(&self) -> &Arc<NativeCodingJob> {
        &self.job
    }

    /// The run's one mode cell, shared with everything that decides a call: a
    /// mode stated here is the mode the next call resolves.
    pub(crate) fn mode_cell(&self) -> ModeCell {
        self.mode.clone()
    }

    /// The session's operator: the one attached to the run, and the one whose
    /// "yes for this run" store every lane of the run records into.
    pub(crate) fn operator(&self) -> &Arc<SessionOperator> {
        &self.operator
    }

    fn inputs(&self) -> MutexGuard<'_, Option<TurnInputs>> {
        self.inputs.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The job request a session's run is created with: the session's name as the
/// job's id and its task label, and the write scope its folder's shape allows.
///
/// `request.task` is a real, stable label — every helper brief and the tool
/// advertisement read it — and never the first message: a prepared run's first
/// message is `run_next_turn`.
fn request(session: &str, shape: &WorkspaceShape) -> Result<CodingJobRequest, CodingError> {
    let job = JobId::parse(session).map_err(CodingError::owner)?;
    let task = format!("session {session}");
    CodingJobRequest::new(job, task, write_scope(shape))
}

/// The paths a run of this shape may write: the folder itself when nothing holds
/// its lease, and nothing when another owner does or the folder cannot be a
/// root at all — a run that writes nothing is the read-only root the core
/// already knows.
fn write_scope(shape: &WorkspaceShape) -> Vec<String> {
    match shape {
        WorkspaceShape::Writable => vec![".".to_owned()],
        WorkspaceShape::ReadOnlyFiles { .. } | WorkspaceShape::NoWorkspace { .. } => Vec::new(),
    }
}
