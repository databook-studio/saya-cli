// The full-screen surface's launch: the session's state decides the model, the
// line front's own composition opens the one run, and the loop runs on the real
// terminal until the operator leaves.
//
// Nothing here is a second session: the folder rule, the grants, the budget, the
// seed and the record's own resume decision are the line front's
// (`line_chat::open_surface`), and the builders below state only what the
// surface itself needs — the model it opens on, the facts every frame is painted
// with, the record its run is opened for, and the parts it is composed from.
use std::sync::Mutex;
use std::time::Instant;

use crate::front::tui::direct_run::DirectDeps;
use crate::front::tui::run_draw::Facts;
use crate::front::tui::{LiveRuntime, Model, OpsDeps, Surface, channel, run as run_loop};
use crate::interactive::session::line_chat::open_surface;

/// What a launch hands the surface: the session its run is composed from, and
/// the record the one opener reads the resume decision from.
pub(super) struct Composed {
    pub(super) parts: crate::interactive::session::line_chat::LineSession,
    pub(super) record: saya_store::RedactedSession,
}

/// Everything the surface is started with: the session it opens, the model it
/// opens on, what its commands need, and the configuration and session state a
/// reload moves. A struct rather than ten positional parameters, so the launch
/// reads by name.
pub(super) struct SurfaceLaunch {
    pub(super) composed: Composed,
    pub(super) model: Model,
    pub(super) facts: Facts,
    pub(super) direct: Arc<DirectDeps>,
    pub(super) commands: crate::slash::user_commands::UserCommands,
    pub(super) metrics: Option<Arc<crate::local_metrics::LocalMetrics>>,
    pub(super) entered: Instant,
    /// The launch's own options: `/reload`'s same-launch re-read, and the flow
    /// the guided setup runs.
    pub(super) options: GlobalOptions,
    /// The configuration in force, shared with `direct`.
    pub(super) runtime: Arc<LiveRuntime>,
    /// The session's state, shared with the launch: a reload re-seeds it.
    pub(super) state: Arc<Mutex<SessionState>>,
}

/// Runs the surface over the launch's parts, and nothing on the terminal once it
/// returns: the run is composed and opened on this thread, and the loop enters
/// and leaves the terminal itself.
pub(super) fn run_surface(launch: SurfaceLaunch) -> Result<(), Box<dyn std::error::Error>> {
    let SurfaceLaunch {
        composed,
        model,
        facts,
        direct,
        commands,
        metrics,
        entered,
        options,
        runtime: live,
        state,
    } = launch;
    let Composed { parts, record } = composed;
    let (msgs, queue) = channel();
    let front = Arc::new(crate::front::tui::run_front::Forward::new(msgs.clone()));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async move {
        let opened = open_surface(
            parts,
            &record,
            Arc::clone(&front) as Arc<dyn crate::chat::ChatFront>,
        )
        .await
        .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
        let allow_bypass = model.allow_bypass;
        let deps = Arc::new(OpsDeps {
            chat: crate::chat::ChatOps::new(opened.run_port(), opened.context(allow_bypass)),
            commands: commands.clone(),
            runtime: Arc::clone(&live),
        });
        run_loop(Surface {
            model,
            msgs,
            queue,
            front,
            session: opened.chat(),
            ops: opened.context(allow_bypass),
            deps,
            direct,
            runtime: live,
            state,
            options,
            facts,
            metrics,
            entered,
        })
        .await
        .map_err(|error| -> Box<dyn std::error::Error> { error.into() })
    })
}

/// The session this surface's run is composed from: the session's own folder,
/// mode, overrides, grants, budget and seed — every fact the plain loop's first
/// message states, stated here once — and the record that message's opener reads.
pub(super) fn surface_parts(
    state: &crate::interactive::session::state::SessionState,
    session: &crate::interactive::session::runtime::SessionRuntime,
    runtime: &crate::config::runtime::RuntimeConfig,
    state_db: &saya_store::SqliteStateStore,
    format: crate::render::RenderFormat,
) -> Composed {
    // The session's live task list: the cell the run's `checklist` writes
    // through, re-seeded from the record exactly as the plain loop re-seeds it.
    let tasks = session.universe().tasks();
    tasks.replace(state.task_list.clone());
    let folder = session
        .root()
        .map(std::path::Path::to_path_buf)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    Composed {
        record: surface_record(state),
        parts: crate::interactive::session::line_chat::LineSession {
            session: state.id.clone(),
            folder,
            runtime: Arc::new(runtime.clone()),
            store: state_db.clone(),
            mode: state
                .approval_mode
                .parse()
                .unwrap_or(saya_agent::ApprovalPolicy::Ask),
            overrides: state.prompt_overrides(),
            format,
            // The surface renders its turns itself: no sink is stated.
            sink: None,
            budget: crate::plugins::database::DatabaseBudget::of(runtime),
            can_prompt: true,
            grants: session.policy().grants().clone(),
            state_root: None,
            cancellation: None,
            tasks: Some(tasks),
            facts: Some(crate::chat::SessionFacts::of(
                state,
                session.universe().workspace(),
            )),
            seed: state.replayed_pairs(),
        },
    }
}

/// The record this launch resumed, as the one opener reads it: the session's own
/// record, with its compaction summary restored — the save drops it (the chat
/// session states it), and what a run may carry depends on it.
pub(super) fn surface_record(
    state: &crate::interactive::session::state::SessionState,
) -> saya_store::RedactedSession {
    let mut record = state.redacted();
    record.compaction_summary = state.compaction_summary.clone();
    record
}

/// The model the surface opens on: the session's mode, its bypass gate, the
/// database it talks to, and whether the record is a runless one (`04` §8: a
/// record with no link is a 0.5.0 record, which opens read-only and prepares a
/// run on the next message).
pub(super) fn surface_model(state: &crate::interactive::session::state::SessionState, fresh: bool) -> Model {
    Model {
        mode: crate::interactive::session::permissions::mode::current(state).unwrap_or_default(),
        allow_bypass: state.allow_bypass,
        database: state.profile.clone(),
        read_only_record: !fresh && state.run.is_none(),
        ..Model::default()
    }
}

/// The facts every frame is painted with: the workspace this session bound, the
/// posture its launch derived — the same view the old status bar reads, so the
/// two surfaces state it in one place — the trust question when it is still
/// open, and the token column the launch resolved once from its environment.
pub(super) fn surface_facts(
    state: &crate::interactive::session::state::SessionState,
    trust_pending: bool,
    scheme: crate::front::tui::theme::Scheme,
) -> Facts {
    let view = crate::interactive::session_prompt::status_segments(state);
    Facts {
        untrusted: trust_pending,
        workspace: view.workspace_root,
        model: view.model,
        host_unsandboxed: view.host_composed,
        sharing: view.sharing_on,
        deny: view.denied_programs,
        scheme,
        ..Facts::default()
    }
}
