//! The `/compact` execution shared by both surfaces: plan, run the bounded
//! summariser, validate, apply or leave untouched.
//!
//! Headless calls [`run`] directly (it owns a `block_on` seam); the TUI
//! drives [`start`] + [`poll`] so the event loop never blocks on the model.

use super::session_compact::{self, COMPACT_MIN_TURNS};
use super::session_compact_call;
use super::session_state::SessionState;
use super::session_universe::SessionUniverse;
use crate::agent::provider;
use crate::config::runtime::RuntimeConfig;
use crate::interactive::tui::types::{App, CompactOutcome};
use std::sync::Arc;

/// The outcome of one `/compact` invocation: the message to show, and the
/// compaction call's usage (kept apart from the answering total). On success
/// the summary and compacted-turn count ride along so the TUI poll path can
/// apply the same working-memory change to the live session the worker
/// applied to its clone — one operation, two triggers, byte-identical.
pub(crate) struct CompactResult {
    pub(crate) message: String,
    pub(crate) failed: bool,
    pub(crate) usage: Option<saya_agent::TokenUsage>,
    pub(crate) summary: Option<String>,
    pub(crate) compacted_turns: usize,
    /// `true` when the worker ran as an automatic firing: the poll path
    /// prefixes the manual strings and applies the no-retry failure policy.
    pub(crate) automatic: bool,
}

/// Runs `/compact` to completion: plan, summarise, validate, apply or leave
/// the session exactly as it was. Pure against `state` except on validated
/// success; the transcript is never rewritten.
///
/// `automatic` names the trigger only — the operation is identical either way
/// (one code path, two triggers), and the message stays the manual one: the
/// poll path prefixes the automatic strings, beside the single operation.
pub(crate) async fn run(
    runtime: &RuntimeConfig,
    state: &mut SessionState,
    _session: &SessionUniverse,
) -> CompactResult {
    run_with_trigger(runtime, state, _session, false).await
}

/// The shared operation with the trigger named: [`run`] is the manual
/// spelling, the automatic path passes `automatic: true`. Kept as one
/// function — never two copies — so the paths cannot drift.
pub(crate) async fn run_with_trigger(
    runtime: &RuntimeConfig,
    state: &mut SessionState,
    _session: &SessionUniverse,
    automatic: bool,
) -> CompactResult {
    let no_work = |message: String| CompactResult {
        message,
        failed: false,
        usage: None,
        summary: None,
        compacted_turns: 0,
        automatic,
    };
    let failed = |message: String, usage: Option<saya_agent::TokenUsage>| CompactResult {
        message,
        failed: true,
        usage,
        summary: None,
        compacted_turns: 0,
        automatic,
    };
    let history = state.provider_history();
    let Some(plan) = session_compact::plan(&state.turns, &history) else {
        return no_work("Nothing to compact: the conversation is short enough already.".into());
    };
    if state.turns.len() <= COMPACT_MIN_TURNS {
        return no_work("Nothing to compact: the conversation is short enough already.".into());
    }
    let overrides = state.prompt_overrides();
    let ai = crate::agent::runtime::effective_ai(&runtime.resolved.ai, &overrides);
    let resolver = runtime.secret_resolver();
    let provider = match provider::build(&ai, &resolver) {
        Ok(provider) => provider,
        Err(error) => {
            return failed(
                session_compact::failure_message(&format!("the summariser errored: {error}")),
                None,
            );
        }
    };
    match session_compact_call::summarise(&*provider, &ai.model, &plan).await {
        Ok(outcome) => match session_compact::apply(state, &plan, &outcome.summary) {
            Ok(()) => CompactResult {
                message: session_compact::success_message(plan.compacted_turns, &outcome.summary),
                failed: false,
                usage: outcome.usage,
                summary: Some(outcome.summary),
                compacted_turns: plan.compacted_turns,
                automatic,
            },
            Err(reason) => failed(session_compact::failure_message(&reason), outcome.usage),
        },
        Err(reason) => failed(session_compact::failure_message(&reason), None),
    }
}

/// Starts the `/compact` worker on the TUI: a short-circuit message when
/// there is nothing to do or no history, otherwise a detached thread running
/// [`run`] to completion. Never blocks the frame loop.
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
    let (tx, rx) = std::sync::mpsc::channel();
    app.compact_task = Some(rx);
    app.request.started = Some(std::time::Instant::now());
    app.request.activity = Some("compacting".into());
    std::thread::spawn(move || {
        let result = super::session_resume::block_on(run_with_trigger(
            &runtime, &mut owned, &universe, automatic,
        ));
        let _ = tx.send(CompactOutcome {
            message: result.message,
            failed: result.failed,
            usage: result.usage,
            automatic: result.automatic,
            summary: result.summary,
            compacted_turns: result.compacted_turns,
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
    use super::auto_compact::{auto_failure_message, auto_success_message};
    use crate::interactive::tui::transcript::BlockKind;
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
                                && session_compact::validate(summary, &plan.pinned).is_ok()
                        });
                        if applicable
                            && let Some(plan) =
                                session_compact::plan(&state.turns, &state.provider_history())
                            && session_compact::apply(state, &plan, summary).is_ok()
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
                    if let Some(summary) = outcome.summary.as_deref() {
                        let plan_check =
                            session_compact::plan(&state.turns, &state.provider_history());
                        let applicable = plan_check.is_some_and(|plan| {
                            plan.compacted_turns == outcome.compacted_turns
                                && session_compact::validate(summary, &plan.pinned).is_ok()
                        });
                        if applicable
                            && let Some(plan) =
                                session_compact::plan(&state.turns, &state.provider_history())
                            && session_compact::apply(state, &plan, summary).is_ok()
                        {
                            state.auto_compact_failed = false;
                        }
                    }
                    app.transcript.push(BlockKind::System, outcome.message);
                }
                true
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => false,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                app.request.started = None;
                app.request.activity = None;
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
