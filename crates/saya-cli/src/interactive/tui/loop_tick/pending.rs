//! Draining one queued prompt once the request ends.

use super::super::application::SecondSqlDecision;
use super::super::dispatch::Dispatch;
use super::super::session_save::queue_session_save;
use super::super::setup_apply;
use super::super::sql_task;
use super::super::startup_readiness::StartupReadiness;
use super::super::terminal_handoff::{TerminalModes, run_guided_setup};
use super::super::transcript::BlockKind;
use super::super::types::App;
use super::resume::adopt_resumed_session;
use crate::cli::GlobalOptions;
use crate::config::runtime::RuntimeConfig;
use crate::interactive::compact_task;
use crate::interactive::session_runtime::SessionRuntime;
use crate::interactive::session_state::SessionState;
use crate::render::RenderFormat;
use saya_store::FsSessionStore;
use std::io;
use std::sync::Arc;

/// The `/setup` handoff inputs, threaded from `session::run` (which owns the
/// terminal guard) and the launch site (which owns the CLI options). A
/// bundle rather than two more positional parameters, so the call sites read
/// by name. The name still names its first caller; `/reload` reads the same
/// `options` for its `ReloadInputs::from_launch`, so the launch options ride
/// one bundle.
pub(crate) struct SetupInputs<'a> {
    /// The TUI's terminal modes, so the handoff can release and reacquire
    /// them; the live loop passes its `TerminalGuard`.
    pub(crate) modes: &'a mut dyn TerminalModes,
    /// The launch's CLI options, for the wizard's real flow options and
    /// `/reload`'s same-launch-options re-read.
    pub(crate) options: &'a GlobalOptions,
}

// Queued prompts (submitted while busy) wait until the request ends.

/// Swaps the app onto a reloaded runtime and refreshes everything derived
/// from it: the profile list (the first screen's database line, the
/// completion popup), the splash's startup facts, and the @-references.
/// Shared by the `/reload` arm and the post-setup apply — one swap, one
/// refresh, never a copy. The swap is observed by the NEXT loop iteration
/// (each one derives its `&RuntimeConfig` from `app.runtime`), never by the
/// tick that is still borrowing the old one.
pub(crate) fn apply_reloaded_runtime(
    app: &mut App,
    state: &mut SessionState,
    new: Box<RuntimeConfig>,
) {
    app.profiles = new.connections.profiles.keys().cloned().collect();
    app.startup = StartupReadiness::from_runtime(&new);
    app.runtime = Arc::new(*new);
    app.reload_at_refs(state);
}

pub(crate) fn tick_pending(
    app: &mut App,
    state: &mut SessionState,
    runtime: &RuntimeConfig,
    store: &FsSessionStore,
    format: RenderFormat,
    session: &mut SessionRuntime,
    setup: &mut SetupInputs<'_>,
) -> io::Result<()> {
    if !app.is_busy()
        && let Some(line) = app.pending.take()
    {
        let id_before = state.id.clone();
        let outcome = super::super::dispatch::dispatch(
            &line,
            &mut app.transcript,
            &app.profiles,
            state,
            runtime,
            store,
            &app.state_db,
            format,
            &mut app.last_query,
            &app.captured,
            app.agent_captures.gap,
            session,
        );
        match outcome {
            Dispatch::Quit => {
                // An in-flight run is not orphaned by a quit: the quit is
                // refused until the run is cancelled or finished.
                if app.try_quit() {
                    app.should_quit = true;
                }
            }
            // A command may have switched profiles; refresh @-references.
            Dispatch::Handled => app.reload_at_refs(state),
            Dispatch::Agent(prompt) => {
                app.start_agent(prompt, state, session);
            }
            Dispatch::OpenSessionPicker => app.open_session_picker(store),
            Dispatch::OpenInvestigationPicker => app.open_investigation_picker(),
            Dispatch::Compact => {
                compact_task::start(&mut *app, state);
            }
            Dispatch::SetColumns(arg) => app.set_visible_columns(arg),
            Dispatch::RunPanel {
                goal,
                allow,
                budget,
            } => app.start_run_panel(goal, allow, budget, format, state),
            Dispatch::SqlTask(task) => {
                // One SQL command in flight at a time. The queued-prompt
                // gate (`!is_busy()`, which now covers SQL tasks) is the
                // primary defence: a second command submitted while one
                // runs is held until the first finishes. This guard is the
                // backstop — should a SqlTask reach the handler while one
                // is already running, refuse rather than silently drop the
                // first result. It also holds one process-wide worker
                // permit (detached workers count against the same cap);
                // when all are held the dispatch is refused, never queued.
                match app.admit_second_sql() {
                    SecondSqlDecision::Start(permit) => {
                        let started = std::time::Instant::now();
                        let cancellation = saya_agent::CancellationToken::new();
                        // Share the existing `Arc<RuntimeConfig>` instead of
                        // deep-cloning the whole config (resolved plaintext
                        // secrets included) onto a detached thread per
                        // command.
                        app.sql_task = Some((
                            sql_task::spawn(
                                permit,
                                Arc::clone(&app.runtime),
                                task.clone(),
                                cancellation.clone(),
                            ),
                            task,
                            started,
                            cancellation,
                        ));
                        // Reuse the agent status fields so the status bar
                        // (which reads them) shows "running query Ns" with
                        // a spinner while the query runs. A
                        // SQL task and an agent stream never run
                        // concurrently — the gate prevents dispatch while
                        // either is busy — so these fields are free to reuse.
                        app.request.started = Some(started);
                        app.request.activity = Some("query".into());
                    }
                    SecondSqlDecision::Reject(message) => {
                        app.transcript.push(BlockKind::System, message)
                    }
                }
            }
            // The saved-investigation replay takes the same one-query-at-a-
            // time resource: admit (spawn off-thread, name the
            // investigation in the bar) or refuse — never a silent
            // replacement of the running query or replay.
            Dispatch::ReplayTask(task) => app.start_replay(task),
            // `/setup` — the guided-setup handoff, run here (the loop thread)
            // so the wizard owns stdin/stdout in the normal screen and the
            // TUI event loop simply waits. The busy gate above guarantees
            // this never runs under a streaming request, query, compaction,
            // or replay. A lost terminal (the reacquire failed) is an error:
            // the loop must stop — the error returns through `run`. A closed
            // wizard's configuration is applied by the same reload `/reload`
            // runs (`setup_apply`).
            Dispatch::RunSetup => {
                let mut flow = crate::setup::flow_options::FlowOptions::real(setup.options.clone());
                let outcome = run_guided_setup(&mut *setup.modes, &mut flow);
                setup_apply::apply_setup_outcome(outcome, app, state, setup.options)?;
            }
            // `/reload` — re-read the configuration with the SAME launch
            // options and, only on success, swap the app's runtime handle.
            // The busy gate above guarantees a reload never overlaps a
            // running stream, query, compaction, or replay — it waits in the
            // queue like every command. A failed build changes nothing and
            // says the previous configuration is still in use. Data sharing,
            // host commands, grants and the approval mode are classified
            // pending by the shared plan and never move.
            Dispatch::Reload => {
                let inputs = crate::config::reload::ReloadInputs::from_launch(setup.options);
                let old = Arc::clone(&app.runtime);
                match crate::config::reload::reload_runtime(&inputs, &old, state) {
                    crate::config::reload::ReloadOutcome::Reloaded { new, text } => {
                        apply_reloaded_runtime(app, state, new);
                        app.transcript.push(BlockKind::System, text);
                    }
                    crate::config::reload::ReloadOutcome::Failed(text) => {
                        app.transcript.push(BlockKind::Error, text);
                    }
                }
            }
        }
        // A `/resume` swapped the session: the engine side too, so the
        // app's universe is the resumed session's, not the old one's.
        // The live list is seeded with the swap, so the next turn sees
        // what the resumed record carried. The old session's ephemeral
        // query state retires with the swap (a successful resume is a
        // context change); a refused or failed resume keeps state.id
        // and changes nothing.
        if state.id != id_before {
            adopt_resumed_session(app, state, session);
        }
        queue_session_save(&mut *app, store, state);
    }
    Ok(())
}
