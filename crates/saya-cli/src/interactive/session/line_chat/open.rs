//! Composing and opening the line session's run.
//!
//! Everything the run is opened with comes from what the line session already
//! resolved: the folder it was started in, the databases its profiles name, its
//! approval mode, its privacy and memory settings. The composition itself is the
//! chat layer's ([`RunComposition`]) — this module only states those facts, so
//! the surface above it knows no epoch, slot or lease.
//!
//! The run is composed and opened **before the first message**: after `open`
//! returns, the run holds its provider, toolset, plugin sections and the
//! folder's lease, and the session's first message is its first turn.
//!
//! **One statement of a run's facts.** [`session_run`] builds the run the
//! session states, and the rollover a compaction asks for builds the next one
//! the same way: the same folder, the same mode, the same databases — only the
//! job name differs, which is what makes the run that follows a new job.
//!
//! **One opener.** [`open_run`] composes and opens a run over a host and a front
//! its caller stated, so the first run, the rollover's next one, the run a
//! resumed record names and the one a read-only record prepares are one path:
//! what a record's link decides ([`super::begin::decided`]) is the only thing
//! that differs, never how a run is opened.

use std::path::PathBuf;
use std::sync::Arc;

use saya_agent::{AgentEventSink, ApprovalPolicy, CancellationToken};
use saya_store::{RedactedSession, SqliteStateStore};

use super::line::Line;
use super::run::Run;
use crate::agent::turn_config::PromptOverrides;
use crate::agent::turn_config::query_data_allowed_for_endpoint;
use crate::agent::turn_context::memory_reachable;
use crate::agent::turn_inputs::prepare_turn;
use crate::chat::{ChatFront, RunComposition, SessionRun, WorkspaceShape};
use crate::commands::coding::CodingApplication;
use crate::config::runtime::RuntimeConfig;
use crate::front::LineFront;
use crate::interactive::turn_summary::tally_handle;
use crate::local_metrics::LocalMetrics;
use crate::plugins::database::DatabaseBudget;
use crate::render::RenderFormat;

/// The facts a line session resolves before its run can be composed.
#[derive(Clone)]
pub(crate) struct LineSession {
    /// The session's own name: the run's job id, and the label its request
    /// states. A saved record's link names the same job.
    pub(crate) session: String,
    /// The folder the session bound: the run's root. `None` for a session that
    /// bound none — the trust question answered `n`, a launch from the OS
    /// temporary directory — whose run is composed with no workspace at all
    /// rather than over the directory the process stands in.
    pub(crate) folder: Option<PathBuf>,
    pub(crate) runtime: Arc<RuntimeConfig>,
    pub(crate) store: SqliteStateStore,
    /// The session's approval mode: the consent to open the run, and the mode
    /// its one cell starts in.
    pub(crate) mode: ApprovalPolicy,
    /// The session's own overrides — the profiles it loaded, which decide the
    /// connections the run's database plugin activates on.
    pub(crate) overrides: PromptOverrides,
    pub(crate) format: RenderFormat,
    /// The sink this surface renders a turn through, when it brings its own
    /// instead of the terminal's. The one-shot `ask` brings the answer sink the
    /// old turn body printed on, so its text and JSON stay byte-identical; the
    /// plain loop brings none and renders on the terminal.
    pub(crate) sink: Option<Arc<dyn AgentEventSink>>,
    /// The database budget a turn of this session spends once it uses the
    /// database: the chat's `[limits.database]`, or the one-shot ask's override
    /// of it (`[limits.ask]`, mapped for one release).
    pub(crate) budget: DatabaseBudget,
    /// Whether this surface may put a question to somebody (the loop's
    /// live-terminal fact).
    pub(crate) can_prompt: bool,
    /// The grants the launch stated (`--allow`): the session's seed store, whose
    /// held tokens pre-answer the calls a mode would have asked about. The run's
    /// host is told them, so the families they cover are advertised exactly as
    /// the session advertised them.
    pub(crate) grants: saya_agent::SessionGrants,
    /// The coding state root this session's job records under, when the surface
    /// states one instead of taking the configured root: a one-shot ask keeps no
    /// records inside the folder it asks about, so its one-turn job records
    /// elsewhere rather than refusing the question.
    pub(crate) state_root: Option<PathBuf>,
    /// The question's own stop, when the surface drove this session from one: a
    /// one-shot ask hands the token its question already answers to, so a stop
    /// the ask latches — its elapsed budget, the operator's signal — also ends
    /// the call waiting at the terminal. The plain loop states none: its turns
    /// are its own.
    pub(crate) cancellation: Option<CancellationToken>,
    /// The session's live task list: the cell the run's `checklist` writes
    /// through, synced back into the session record after every turn.
    pub(crate) tasks: Option<crate::interactive::session::tasks::SessionTasks>,
    /// The session facts the run's requests carry: the compaction narrative and
    /// the continuation record the session's loop holds. Written at this
    /// front's compaction boundary, read where a request is assembled.
    pub(crate) facts: Option<Arc<crate::chat::SessionFacts>>,
    /// The conversation the run begins with, oldest pair first: the pairs the
    /// session's record keeps ahead of the compaction boundary, so a resumed
    /// session replays its kept tail exactly as the run before it did. Empty —
    /// a session that has said nothing yet — is a run with no history.
    pub(crate) seed: Vec<(String, String)>,
}

/// The same composition, driven by a surface that already runs on a runtime:
/// one line session, composed and prepared exactly as the plain loop's first
/// message prepares it, owned by that runtime instead of one of its own.
///
/// The one-turn surface states its own state root here: an ask records nothing
/// inside the folder it asks about, so when the configured root would land
/// there the session states a private root of its own instead — the question
/// then reaches its provider and reports what it found, rather than being
/// refused for where the job's records would have gone.
///
pub(super) async fn one(mut parts: LineSession) -> Result<Line, String> {
    parts.state_root = super::one_shot::state_root(&parts)?;
    assemble(parts, true).await
}

/// Builds the host the session's run is composed over, and states the run's
/// facts — asking nobody: the session's mode is the consent.
async fn assemble(parts: LineSession, one_shot: bool) -> Result<Line, String> {
    let hosted = host(&parts).await?;
    if one_shot {
        super::one_shot::state_on(&hosted.application);
    }
    // The run's own counters, printed after every settled turn: the same
    // collector the answering turn hands its metrics sink, and the stop
    // instant a Ctrl-C is measured from. Its cancellation token is the scope's
    // own — the run keeps the one it was opened with — and a surface that
    // brought the question's token states it, so the stop the question latched
    // is the stop this front waits on.
    let (front, metrics) = front_for(&parts);
    let (run, application, shape) = open_run(
        &parts,
        hosted,
        Arc::clone(&front) as Arc<dyn ChatFront>,
        metrics.clone(),
    )
    .await?;
    Ok(Line::new(application, parts, front, shape, metrics, run))
}

/// The host a run is composed over: the application it composes on, and the
/// shape of the folder it opened over.
pub(super) struct Hosted {
    pub(super) application: Arc<CodingApplication>,
    pub(super) shape: WorkspaceShape,
}

/// The host `parts` state: the folder's shape first, because it decides which
/// root the application opens over.
pub(super) async fn host(parts: &LineSession) -> Result<Hosted, String> {
    let (application, shape) = super::session_host::open_host(parts).await?;
    Ok(Hosted { application, shape })
}

/// The run a surface states over `hosted`, composed and opened, with `front`
/// receiving everything the session says. The application the run composed on
/// comes back with it: a surface's own successor runs compose over that one.
pub(super) async fn open_run(
    parts: &LineSession,
    hosted: Hosted,
    front: Arc<dyn ChatFront>,
    metrics: Option<Arc<LocalMetrics>>,
) -> Result<(Run, Arc<CodingApplication>, WorkspaceShape), String> {
    // The launch's own seeds ride the host the run composes over: a held token
    // pre-answers the calls its family would have asked about, so the mode's
    // advertisement states the family the session stated.
    hosted.application.with_launch_grants(parts.grants.clone());
    let stated = session_run(
        parts,
        Arc::clone(&front),
        parts.session.clone(),
        &hosted.shape,
    )
    .await?;
    let composed = RunComposition::compose_seeded(hosted.application, stated, parts.seed.clone())
        .await
        .map_err(|error| error.to_string())?;
    // The composition states the operator on the host, which only the host's one
    // handle can do: the handle it was given is read back from the job it made,
    // so a call of this run is put to the person at this terminal.
    let application = composed.job().application();
    composed.open().await.map_err(|error| error.to_string())?;
    let run = Run::opened(composed, front, metrics).await;
    Ok((run, application, hosted.shape))
}

/// The run the full-screen surface drives: the line loop's own composition —
/// the same folder rule, the same grants, the same budget, the same seed — with
/// the record the launch resumed read the same way the line loop reads it, so a
/// record whose link attaches, prepares its next epoch, opens read-only or is
/// refused opens that way on this surface too.
pub(crate) async fn open_surface(
    parts: LineSession,
    record: &RedactedSession,
    front: Arc<dyn ChatFront>,
) -> Result<Opened, String> {
    // The front the run composes on is also the front every run after a
    // rollover composes on, so the line keeps its own handle to it.
    let composing = Arc::clone(&front);
    match super::begin::decided(&parts, record, front, None).await? {
        // The whole line, not just the run: the surface holds the session's
        // transcript, compaction and rollover, so a `/clear` on the live run is
        // the line front's own clear.
        super::begin::Decided::Open(open) => {
            let super::begin::OpenRun {
                run,
                application,
                shape,
                ..
            } = *open;
            Ok(Opened {
                line: Line::surface(application, parts, composing, shape, None, run),
            })
        }
        // A job this release takes no further turn on states its one sentence;
        // the surface opens nothing and the launch reports it.
        super::begin::Decided::Refused { sentence } => Err(sentence),
    }
}

/// A run one surface outside the line loop drives, held for as long as that
/// surface lives: the run owns the folder's lease, so it must outlive every
/// handle taken from it.
pub(crate) struct Opened {
    line: Line,
}

impl Opened {
    /// The line session this run is the first of: the surface keeps it for as
    /// long as it drives the run.
    pub(crate) fn into_line(self) -> Line {
        self.line
    }
}

/// The surface every turn prints on, and the counters its scope records into:
/// stated once for the composed run and for the resumed one.
pub(super) fn front_for(parts: &LineSession) -> (Arc<LineFront>, Option<Arc<LocalMetrics>>) {
    let config: &RuntimeConfig = &parts.runtime;
    let stop = parts.cancellation.clone().unwrap_or_default();
    let scope = match parts.cancellation.is_some() {
        true => crate::agent::question_scope::QuestionScope::ask(config, stop),
        false => crate::agent::question_scope::QuestionScope::session(config, stop),
    };
    let metrics = scope.metrics();
    let front = match parts.sink.clone() {
        Some(sink) => LineFront::over(sink, parts.can_prompt, tally_handle()),
        None => LineFront::new(parts.format, parts.can_prompt, tally_handle()),
    };
    (Arc::new(front.with_scope(scope)), metrics)
}

/// The run a session states — the first one, and every one a rollover prepares
/// in its place: the same folder, the same mode and the same resolved
/// databases, under the job name the session is on.
pub(super) async fn session_run(
    parts: &LineSession,
    front: Arc<dyn ChatFront>,
    name: String,
    shape: &WorkspaceShape,
) -> Result<SessionRun, String> {
    let config: &RuntimeConfig = &parts.runtime;
    // The turn's injectable inputs: the provider the run is configured with and
    // the live registry its database plugin activates on. The provider object
    // is the run's own (composed at open), so this one is only how the registry
    // and the privacy facts are resolved through the one composition path.
    let inputs = prepare_turn(config, &parts.overrides, parts.can_prompt, None)
        .await
        // A setup failure is worded the way the `ask` has always worded it: an
        // unresolved secret names the step that resolves it, at the one seam
        // both the question and the plain loop pass through.
        .map_err(|error| crate::agent::turn_config::with_next_step(error).to_string())?;
    let allow_query_data = query_data_allowed_for_endpoint(
        inputs.ai.provider,
        inputs.ai.base_url.as_deref(),
        inputs.ai.allow_data_sharing,
    );
    let memory = config.resolved.memory.mode;
    // What of the session's connections did not open. The primary is the one a
    // surface must state: the run is a session with no database, and the
    // operator reads why, once, in the session's own voice — the transcript's
    // line on this surface, the loop's on the other.
    let configured = inputs.registry.primary_name().to_owned();
    let skipped = inputs
        .failures
        .iter()
        .find(|(name, _)| *name == configured)
        .cloned();
    if let Some((name, reason)) = &skipped {
        front.emit(crate::chat::ChatEvent::Notice {
            // One row at the narrowest terminal the surface is drawn at (80
            // columns leave 72 cells of transcript): the profile, then the
            // error in the words the connector stated.
            message: format!("cannot open profile '{name}': {reason}"),
        });
    }
    // The provider the run's post-turn learning pass calls: only a session whose
    // memory mode is on has one, so a memory-off run extracts nothing and says
    // nothing.
    let learning: Option<Arc<dyn saya_agent::ChatProvider>> = match memory {
        saya_config::MemoryMode::Off => None,
        _ => Some(Arc::from(inputs.provider)),
    };
    Ok(SessionRun {
        front,
        session: name,
        mode: parts.mode,
        budget: parts.budget,
        shape: shape.clone(),
        connections: inputs.registry,
        allow_query_data,
        memory,
        memory_reachable: memory_reachable(true, allow_query_data),
        capture: None,
        // Memory off learns nothing; with memory on, the run learns through the
        // plugin's post-turn hook, on its own provider and model.
        learning,
        tasks: parts.tasks.clone(),
        facts: parts.facts.clone(),
        skipped_primary: skipped.map(|(name, _)| name),
    })
}
