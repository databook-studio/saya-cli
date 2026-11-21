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

use std::path::PathBuf;
use std::sync::Arc;

use saya_agent::{ApprovalPolicy, CancellationToken};
use saya_store::SqliteStateStore;

use super::LineChat;
use crate::agent::runtime::PromptOverrides;
use crate::agent::system_prompt::memory_reachable;
use crate::agent::turn_config::query_data_allowed_for_endpoint;
use crate::agent::turn_inputs::prepare_turn;
use crate::chat::{ChatSession, RunComposition, SessionRun, workspace_shape};
use crate::commands::coding::{CodingApplication, NativeCodingJob};
use crate::config::runtime::RuntimeConfig;
use crate::front::LineFront;
use crate::interactive::turn_summary::tally_handle;
use crate::render::RenderFormat;

/// The facts a line session resolves before its run can be composed.
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
    /// Whether this surface may put a question to somebody (the loop's
    /// live-terminal fact).
    pub(crate) can_prompt: bool,
}

/// Composes and opens the session's run, then hands back the session driving
/// it, already `Ready`.
impl LineChat {
    pub(crate) fn open(parts: LineSession) -> Result<Self, String> {
        compose(parts)
    }
}

fn compose(parts: LineSession) -> Result<LineChat, String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("the session's runtime could not be built: {error}"))?;
    let opened = runtime.block_on(assemble(parts))?;
    let Opened {
        application,
        run,
        session,
        front,
        job,
    } = opened;
    Ok(LineChat {
        _application: application,
        _run: run,
        session,
        front,
        job,
        runtime,
    })
}

/// The composed run and everything above it that must outlive it.
struct Opened {
    application: Arc<CodingApplication>,
    run: RunComposition,
    session: ChatSession,
    front: Arc<LineFront>,
    job: Arc<NativeCodingJob>,
}

/// Builds the application over the session's folder, states the run's facts,
/// composes it and opens it — asking nobody: the session's mode is the consent.
async fn assemble(parts: LineSession) -> Result<Opened, String> {
    let LineSession {
        session,
        folder,
        runtime,
        store,
        mode,
        overrides,
        format,
        can_prompt,
    } = parts;
    let config: &RuntimeConfig = &runtime;
    // The folder is the one the person is sitting in, and a chat takes many
    // turns: the run works in place. A managed checkout runs one turn per run
    // (`chat::link`, the not-resumable refusal), so it is not a chat's root.
    let token = CancellationToken::new();
    let application = CodingApplication::session(
        config.clone(),
        store,
        &folder,
        crate::cli::RootMode::InPlace,
        &token,
    )
    .await
    .map_err(|error| error.to_string())?;
    // The turn's injectable inputs: the provider the run is configured with and
    // the live registry its database plugin activates on. The provider object
    // is the run's own (composed at open), so this one is only how the registry
    // and the privacy facts are resolved through the one composition path.
    let inputs = prepare_turn(config, &overrides, can_prompt, None)
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
    // The run's own counters, printed after every settled turn: the same
    // collector the answering turn hands its metrics sink, and the stop
    // instant a Ctrl-C is measured from. Its cancellation token is the scope's
    // own — the run keeps the one it was opened with.
    let scope =
        crate::agent::question_scope::QuestionScope::session(config, CancellationToken::new());
    let metrics = scope.metrics();
    let front = Arc::new(LineFront::new(format, can_prompt, tally_handle()).with_scope(scope));
    let shape = workspace_shape(&folder, &coding_state_root(config)?);
    let run = SessionRun {
        front: Arc::clone(&front) as Arc<dyn crate::chat::ChatFront>,
        session,
        mode,
        shape,
        connections: inputs.registry,
        allow_query_data,
        memory,
        memory_reachable: memory_reachable(true, allow_query_data),
        capture: None,
        // Memory off learns nothing; with memory on, the run learns through the
        // plugin's post-turn hook, on its own provider and model.
        learning,
    };
    let composed = RunComposition::compose(Arc::clone(&application), run)
        .await
        .map_err(|error| error.to_string())?;
    composed.open().await.map_err(|error| error.to_string())?;
    let job = Arc::clone(composed.job());
    // The run is already open: the port states it and opens nothing, so the
    // session's prepare is the composition's one open.
    let port: Arc<dyn crate::chat::RunPort> = match metrics {
        Some(metrics) => {
            Arc::new(crate::chat::JobPort::opened(Arc::clone(&job)).with_metrics(metrics))
        }
        None => Arc::new(crate::chat::JobPort::opened(Arc::clone(&job))),
    };
    let session = ChatSession::new(port, Arc::clone(&front) as Arc<dyn crate::chat::ChatFront>);
    session.open().await;
    Ok(Opened {
        application,
        run: composed,
        session,
        front,
        job,
    })
}

/// The coding state root the session's runs record themselves under — the same
/// root a `saya run` over this folder uses, read from the one resolver so a
/// folder's lease is looked for where it is held.
fn coding_state_root(config: &RuntimeConfig) -> Result<PathBuf, String> {
    crate::commands::coding::root_mode::session_state_root(config)
        .map_err(|error| error.to_string())
}
