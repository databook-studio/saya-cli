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
use crate::chat::{ChatFront, RunComposition, SessionRun, WorkspaceShape, workspace_shape};
use crate::commands::coding::CodingApplication;
use crate::config::runtime::RuntimeConfig;
use crate::front::LineFront;
use crate::interactive::turn_summary::tally_handle;
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
}

/// Opens the session: composes and opens its run, and hands back the session
/// driving it, already `Ready`.
impl LineChat {
    pub(crate) fn open(parts: LineSession) -> Result<Self, String> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| format!("the session's runtime could not be built: {error}"))?;
        let line = runtime.block_on(assemble(parts))?;
        Ok(LineChat::new(line, runtime))
    }
}

/// The same composition, driven by a surface that already runs on a runtime:
/// one line session, composed and prepared exactly as the plain loop's first
/// message prepares it, owned by that runtime instead of one of its own.
pub(super) async fn one(parts: LineSession) -> Result<Line, String> {
    assemble(parts).await
}

/// Builds the application over the session's folder, states the run's facts,
/// composes it and opens it — asking nobody: the session's mode is the consent.
async fn assemble(parts: LineSession) -> Result<Line, String> {
    let config: &RuntimeConfig = &parts.runtime;
    let token = CancellationToken::new();
    let application = CodingApplication::session(
        config.clone(),
        parts.store.clone(),
        &parts.folder,
        crate::cli::RootMode::InPlace,
        &token,
    )
    .await
    .map_err(|error| error.to_string())?;
    // The launch's own seeds ride the host the run composes over: a held token
    // pre-answers the calls its family would have asked about, so the mode's
    // advertisement states the family the session stated.
    application.with_launch_grants(parts.grants.clone());
    // The run's own counters, printed after every settled turn: the same
    // collector the answering turn hands its metrics sink, and the stop
    // instant a Ctrl-C is measured from. Its cancellation token is the scope's
    // own — the run keeps the one it was opened with.
    let scope =
        crate::agent::question_scope::QuestionScope::session(config, CancellationToken::new());
    let metrics = scope.metrics();
    let front = match parts.sink.clone() {
        Some(sink) => LineFront::over(sink, parts.can_prompt, tally_handle()),
        None => LineFront::new(parts.format, parts.can_prompt, tally_handle()),
    };
    let front = Arc::new(front.with_scope(scope));
    // The folder is writable for this session or it is not: the fact is taken
    // before the first run holds the lease, and every run a rollover prepares
    // states it again.
    let shape = workspace_shape(&parts.folder, &coding_state_root(config)?);
    let name = parts.session.clone();
    let stated = session_run(
        &parts,
        Arc::clone(&front) as Arc<dyn ChatFront>,
        name,
        &shape,
    )
    .await?;
    let composed = RunComposition::compose(Arc::clone(&application), stated)
        .await
        .map_err(|error| error.to_string())?;
    composed.open().await.map_err(|error| error.to_string())?;
    let run = Run::opened(composed, &front, metrics.clone()).await;
    Ok(Line::new(application, parts, front, shape, metrics, run))
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
        .map_err(|error| error.to_string())?;
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
    })
}

/// The coding state root the session's runs record themselves under — the same
/// root a `saya run` over this folder uses, read from the one resolver so a
/// folder's lease is looked for where it is held.
fn coding_state_root(config: &RuntimeConfig) -> Result<PathBuf, String> {
    crate::commands::coding::root_mode::session_state_root(config)
        .map_err(|error| error.to_string())
}
