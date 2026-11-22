// The full-screen surface's launch: the session's state decides the model, the
// line front's own composition opens the one run, and the loop runs on the real
// terminal until the operator leaves.
//
// Nothing here is a second session: the folder rule, the grants, the budget and
// the seed are the line front's (`line_chat::open_surface`), and the three
// builders below state only what the surface itself needs — the model it opens
// on, the facts every frame is painted with, and the parts its run is composed
// from.
use std::time::Instant;

use crate::front::tui::direct_run::DirectDeps;
use crate::front::tui::run_draw::Facts;
use crate::front::tui::{Model, OpsDeps, Surface, channel, run as run_loop};
use crate::interactive::session::line_chat::open_surface;


/// Runs the surface over `parts`, and nothing on the terminal once it returns:
/// the run is composed and opened on this thread, and the loop enters and
/// leaves the terminal itself.
pub(super) fn run_surface(
    parts: crate::interactive::session::line_chat::LineSession,
    model: Model,
    facts: Facts,
    direct: Arc<DirectDeps>,
    commands: crate::slash::user_commands::UserCommands,
    metrics: Option<Arc<crate::local_metrics::LocalMetrics>>,
    entered: Instant,
) -> Result<(), Box<dyn std::error::Error>> {
    let (msgs, queue) = channel();
    let front = Arc::new(crate::front::tui::run_front::Forward::new(msgs.clone()));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async move {
        let opened = open_surface(
            parts,
            Arc::clone(&front) as Arc<dyn crate::chat::ChatFront>,
        )
        .await
        .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
        let allow_bypass = model.allow_bypass;
        let deps = Arc::new(OpsDeps {
            chat: crate::chat::ChatOps::new(opened.run_port(), opened.context(allow_bypass)),
            commands: commands.clone(),
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
            facts,
            metrics,
            entered,
        })
        .await
        .map_err(|error| -> Box<dyn std::error::Error> { error.into() })
    })
}

/// The line session this surface's run is composed from: the session's own
/// folder, mode, overrides, grants, budget and seed — every fact the plain
/// loop's first message states, stated here once.
pub(super) fn surface_parts(
    state: &crate::interactive::session::state::SessionState,
    session: &crate::interactive::session::runtime::SessionRuntime,
    runtime: &crate::config::runtime::RuntimeConfig,
    state_db: &saya_store::SqliteStateStore,
    format: crate::render::RenderFormat,
) -> crate::interactive::session::line_chat::LineSession {
    // The session's live task list: the cell the run's `checklist` writes
    // through, re-seeded from the record exactly as the plain loop re-seeds it.
    let tasks = session.universe().tasks();
    tasks.replace(state.task_list.clone());
    let folder = session
        .root()
        .map(std::path::Path::to_path_buf)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    crate::interactive::session::line_chat::LineSession {
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
    }
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
/// two surfaces state it in one place — and the trust question when it is still
/// open.
pub(super) fn surface_facts(state: &crate::interactive::session::state::SessionState, trust_pending: bool) -> Facts {
    let view = crate::interactive::session_prompt::status_segments(state);
    Facts {
        untrusted: trust_pending,
        workspace: view.workspace_root,
        model: view.model,
        host_unsandboxed: view.host_composed,
        sharing: view.sharing_on,
        deny: view.denied_programs,
        ..Facts::default()
    }
}
