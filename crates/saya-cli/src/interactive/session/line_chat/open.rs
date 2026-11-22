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

use std::path::PathBuf;
use std::sync::Arc;

use saya_agent::{AgentEventSink, ApprovalPolicy, CancellationToken};
use saya_store::SqliteStateStore;

use super::LineChat;
use super::line::Line;
use super::run::Run;
use crate::agent::runtime::PromptOverrides;
use crate::agent::system_prompt::memory_reachable;
use crate::agent::turn_config::query_data_allowed_for_endpoint;
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
    /// The folder the session was started in: the run's root.
    pub(crate) folder: PathBuf,
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

/// Opens the session: composes and opens its run, and hands back the session
/// driving it, already `Ready`.
impl LineChat {
    pub(crate) fn open(parts: LineSession) -> Result<Self, String> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| format!("the session's runtime could not be built: {error}"))?;
        let line = runtime.block_on(assemble(parts, super::one_shot::OneShot::No))?;
        Ok(LineChat::new(line, runtime))
    }
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
    assemble(parts, super::one_shot::OneShot::Yes).await
}

/// Builds the host the session's run is composed over, and states the run's
/// facts — asking nobody: the session's mode is the consent.
async fn assemble(parts: LineSession, one_shot: super::one_shot::OneShot) -> Result<Line, String> {
    let (application, shape) = super::session_host::open_host(&parts).await?;
    if one_shot == super::one_shot::OneShot::Yes {
        super::one_shot::state_on(&application);
    }
    // The launch's own seeds ride the host the run composes over: a held token
    // pre-answers the calls its family would have asked about, so the mode's
    // advertisement states the family the session stated.
    application.with_launch_grants(parts.grants.clone());
    composed_over(application, shape, parts).await
}

/// The run a session composes, over a host its surface has already opened: the
/// first run, the rollover's next one, and the run a read-only record's next
/// line prepares all come through here.
pub(super) async fn composed_over(
    application: Arc<CodingApplication>,
    shape: WorkspaceShape,
    parts: LineSession,
) -> Result<Line, String> {
    // The run's own counters, printed after every settled turn: the same
    // collector the answering turn hands its metrics sink, and the stop
    // instant a Ctrl-C is measured from. Its cancellation token is the scope's
    // own — the run keeps the one it was opened with — and a surface that
    // brought the question's token states it, so the stop the question latched
    // is the stop this front waits on.
    let (front, metrics) = front_for(&parts);
    let name = parts.session.clone();
    let stated = session_run(
        &parts,
        Arc::clone(&front) as Arc<dyn ChatFront>,
        name,
        &shape,
    )
    .await?;
    let composed = RunComposition::compose_seeded(application, stated, parts.seed.clone())
        .await
        .map_err(|error| error.to_string())?;
    // The composition states the operator on the host, which only the host's one
    // handle can do: the handle it was given is read back from the job it made,
    // so a call of this run is put to the person at this terminal.
    let application = composed.job().application();
    composed.open().await.map_err(|error| error.to_string())?;
    let run = Run::opened(composed, &front, metrics.clone()).await;
    Ok(Line::new(application, parts, front, shape, metrics, run))
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
        .map_err(|error| crate::agent::runtime::with_next_step(error).to_string())?;
    let allow_query_data = query_data_allowed_for_endpoint(
        inputs.ai.provider,
        inputs.ai.base_url.as_deref(),
        inputs.ai.allow_data_sharing,
    );
    let memory = config.resolved.memory.mode;
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
    })
}
