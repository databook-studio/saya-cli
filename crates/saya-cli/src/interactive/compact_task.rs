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
/// compaction call's usage (kept apart from the answering total).
pub(crate) struct CompactResult {
    pub(crate) message: String,
    pub(crate) failed: bool,
    pub(crate) usage: Option<saya_agent::TokenUsage>,
}

/// Runs `/compact` to completion: plan, summarise, validate, apply or leave
/// the session exactly as it was. Pure against `state` except on validated
/// success; the transcript is never rewritten.
pub(crate) async fn run(
    runtime: &RuntimeConfig,
    state: &mut SessionState,
    _session: &SessionUniverse,
) -> CompactResult {
    let history = state.provider_history();
    let Some(plan) = session_compact::plan(&state.turns, &history) else {
        return CompactResult {
            message: "Nothing to compact: the conversation is short enough already.".into(),
            failed: false,
            usage: None,
        };
    };
    if state.turns.len() <= COMPACT_MIN_TURNS {
        return CompactResult {
            message: "Nothing to compact: the conversation is short enough already.".into(),
            failed: false,
            usage: None,
        };
    }
    let overrides = state.prompt_overrides();
    let ai = crate::agent::runtime::effective_ai(&runtime.resolved.ai, &overrides);
    let resolver = runtime.secret_resolver();
    let provider = match provider::build(&ai, &resolver) {
        Ok(provider) => provider,
        Err(error) => {
            return CompactResult {
                message: session_compact::failure_message(&format!(
                    "the summariser errored: {error}"
                )),
                failed: true,
                usage: None,
            };
        }
    };
    match session_compact_call::summarise(&*provider, &ai.model, &plan).await {
        Ok(outcome) => match session_compact::apply(state, &plan, &outcome.summary) {
            Ok(()) => CompactResult {
                message: session_compact::success_message(plan.compacted_turns, &outcome.summary),
                failed: false,
                usage: outcome.usage,
            },
            Err(reason) => CompactResult {
                message: session_compact::failure_message(&reason),
                failed: true,
                usage: outcome.usage,
            },
        },
        Err(reason) => CompactResult {
            message: session_compact::failure_message(&reason),
            failed: true,
            usage: None,
        },
    }
}

/// Starts the `/compact` worker on the TUI: a short-circuit message when
/// there is nothing to do or no history, otherwise a detached thread running
/// [`run`] to completion. Never blocks the frame loop.
pub(crate) fn start(app: &mut App, state: &SessionState) {
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
        let result = super::session_resume::block_on(run(&runtime, &mut owned, &universe));
        let _ = tx.send(CompactOutcome {
            message: result.message,
            failed: result.failed,
            usage: result.usage,
        });
    });
}

/// Polls the `/compact` worker (non-blocking): on completion folds the usage
/// apart and pushes the message. A failed compaction changes nothing — the
/// worker owned a clone, so the live session was never at risk.
pub(crate) fn poll(app: &mut App, state: &mut SessionState) {
    use crate::interactive::tui::transcript::BlockKind;
    let ready = app
        .compact_task
        .as_ref()
        .is_some_and(|rx| match rx.try_recv() {
            Ok(outcome) => {
                state.usage.record_learning(outcome.usage);
                app.request.started = None;
                app.request.activity = None;
                if outcome.failed {
                    app.transcript.push(BlockKind::Error, outcome.message);
                } else {
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
