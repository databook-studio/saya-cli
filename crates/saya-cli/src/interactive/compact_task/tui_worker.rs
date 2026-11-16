use crate::interactive::session::continuation::compaction::engine::run_with_trigger;
use crate::interactive::session::continuation::compaction::manual as session_compact;
use crate::interactive::session::continuation::digest_resolver;
use crate::interactive::session::state::SessionState;
use crate::interactive::tui::types::{App, CompactOutcome};
use std::sync::Arc;

/// Starts the `/compact` worker on the TUI: a short-circuit message when
/// there is nothing to do or no history, otherwise a detached thread running
/// [`crate::interactive::compact_task::run`] to completion. Never blocks the frame loop.
pub(crate) fn start(app: &mut App, state: &SessionState) {
    start_with_trigger(app, state, false);
}

/// Starts the automatic worker: the same operation as [`start`] with the
/// automatic trigger named. The turn boundary calls this only after the pure
/// decision fired, so this never re-checks the threshold — one decision, one
/// operation.
pub(crate) fn start_automatic(app: &mut App, state: &SessionState) {
    start_with_trigger(app, state, true);
}

fn start_with_trigger(app: &mut App, state: &SessionState, automatic: bool) {
    use crate::interactive::tui::transcript::BlockKind;
    if state.turns.is_empty() {
        app.transcript.push(
            BlockKind::Error,
            String::from("Nothing to compact: there is no conversation yet."),
        );
        return;
    }
    if app.compact_task.is_some() {
        app.transcript.push(
            BlockKind::System,
            String::from("A compaction is already running — wait for it to finish."),
        );
        return;
    }
    let runtime = Arc::clone(&app.runtime);
    let mut owned = state.clone();
    let universe = Arc::clone(&app.session);
    let metrics = app.session_metrics.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    app.compact_task = Some(rx);
    app.request.started = Some(std::time::Instant::now());
    app.request.activity = Some("compacting".into());
    std::thread::spawn(move || {
        let result = crate::interactive::session::lifecycle::resume::block_on(run_with_trigger(
            &runtime,
            &mut owned,
            &universe,
            automatic,
            metrics.as_deref(),
        ));
        let _ = tx.send(CompactOutcome {
            message: result.message,
            failed: result.failed,
            usage: result.usage,
            automatic: result.automatic,
            summary: result.summary,
            compacted_turns: result.compacted_turns,
            source_identity: result.source_identity,
        });
    });
}

/// Polls the `/compact` worker (non-blocking): on completion folds the usage
/// apart and pushes the message. A manual result applies verbatim; an
/// automatic one renders the automatic strings and applies the no-retry
/// policy.
///
/// A successful worker compacted its clone, never the live session — so the
/// poll path applies the same validated summary to the live session before
/// announcing it. The summary already passed validation on the clone over the
/// same history, so a re-validation failure here (a turn landed mid-flight)
/// keeps the live session unchanged and reports the failure honestly rather
/// than announcing a compaction that never happened. A failed automatic
/// compaction sets the session-scoped `auto_compact_failed` flag: the trigger
/// stays silent until `/clear` or a successful manual `/compact` re-arms it —
/// a session that fails to summarise every turn forever would burn the budget
/// it was trying to save. A successful compaction (either trigger) clears the
/// flag, and a successful manual one additionally re-arms the warning path by
/// the same `apply` it shares with the automatic path.
pub(crate) fn poll(app: &mut App, state: &mut SessionState) {
    use crate::interactive::session::continuation::compaction::automatic::{
        auto_failure_message, auto_success_message,
    };
    use crate::interactive::tui::transcript::BlockKind;
    let digest_of = digest_resolver(app.session.workspace());
    let ready = app
        .compact_task
        .as_ref()
        .is_some_and(|rx| match rx.try_recv() {
            Ok(outcome) => {
                state.usage.record_learning(outcome.usage);
                app.request.started = None;
                app.request.activity = None;
                if outcome.automatic {
                    if outcome.failed {
                        // No working-memory change to apply: the worker owned
                        // a clone, so the live session was never at risk.
                        state.auto_compact_failed = true;
                        app.transcript
                            .push(BlockKind::Error, auto_failure_message(&outcome.message));
                    } else if let Some(summary) = outcome.summary.as_deref() {
                        let plan_check =
                            session_compact::plan(&state.turns, &state.provider_history());
                        let applicable = plan_check.is_some_and(|plan| {
                            plan.compacted_turns == outcome.compacted_turns
                                && Some(plan.source_identity) == outcome.source_identity
                                && session_compact::validate(summary, &plan.pinned).is_ok()
                        });
                        if applicable
                            && let Some(plan) =
                                session_compact::plan(&state.turns, &state.provider_history())
                            && session_compact::apply_with_continuation(
                                state, &plan, summary, &digest_of,
                            )
                            .is_ok()
                        {
                            state.auto_compact_failed = false;
                            app.transcript.push(
                                BlockKind::System,
                                auto_success_message(outcome.compacted_turns, summary),
                            );
                        } else {
                            state.auto_compact_failed = true;
                            app.transcript.push(
                                BlockKind::Error,
                                auto_failure_message("the conversation changed while compacting"),
                            );
                        }
                    } else {
                        // Nothing to compact: not a failure, so the trigger
                        // stays armed — the next crossing decides again.
                        app.transcript.push(BlockKind::System, outcome.message);
                    }
                } else if outcome.failed {
                    app.transcript.push(BlockKind::Error, outcome.message);
                } else {
                    let apply_error = if let Some(summary) = outcome.summary.as_deref() {
                        match session_compact::plan(&state.turns, &state.provider_history()) {
                            Some(plan)
                                if plan.compacted_turns == outcome.compacted_turns
                                    && Some(plan.source_identity) == outcome.source_identity =>
                            {
                                match session_compact::apply_with_continuation(
                                    state, &plan, summary, &digest_of,
                                ) {
                                    Ok(()) => {
                                        state.auto_compact_failed = false;
                                        None
                                    }
                                    Err(reason) => Some(reason),
                                }
                            }
                            _ => Some("the conversation changed while compacting".into()),
                        }
                    } else {
                        None
                    };
                    if let Some(reason) = apply_error {
                        app.transcript
                            .push(BlockKind::Error, session_compact::failure_message(&reason));
                    } else {
                        app.transcript.push(BlockKind::System, outcome.message);
                    }
                }
                true
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => false,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                app.request.started = None;
                app.request.activity = None;
                state.auto_compact_failed = true;
                app.transcript.push(
                    BlockKind::Error,
                    session_compact::failure_message("the compaction worker stopped"),
                );
                true
            }
        });
    if ready {
        app.compact_task = None;
    }
}
