//! Background saved-investigation replay (D13): `/investigation run` is the
//! one investigation subcommand that touches a database, so — like the
//! direct-SQL commands — it runs on its own worker thread with its own
//! current-thread tokio runtime, and the event loop polls a non-blocking
//! `mpsc` receiver each tick. The event loop never blocks on a replay; the
//! status bar names the investigation while it runs.
//!
//! The worker runs the shared typed entry (`run_investigation_outcome`, C3)
//! — the same operation the headless `saya investigation run` takes — with
//! its `emit` output captured through the thread-local seam (a thread's
//! capture buffer is its own). Its store is constructed from the same
//! state-DB path the session resolved, so the replay's audit row and review
//! binding are written exactly as a foreground run writes them.
//!
//! Detach, not cancel: Esc/Ctrl+C drop the receiver (`App::detach_replay_task`)
//! and the UI moves on. The worker is never joined and no cancellation token
//! is wired, so the query may keep running server-side; its [`ReplayDone`]
//! lands on a dropped channel and is discarded.

use super::capture::CapturedResult;
use super::transcript::{BlockKind, Transcript};
use super::types::LastQuery;
use crate::cli::InvestigationCommand;
use crate::commands::{Replay, run_investigation_outcome};
use crate::config::runtime::RuntimeConfig;
use crate::render::RenderFormat;
use std::sync::Arc;
use std::sync::mpsc::Receiver;

/// A dispatched replay: the investigation it runs (naming it for the status
/// bar and the detach message) and the exact command the worker executes.
#[derive(Debug, Clone)]
pub(crate) struct ReplayTask {
    pub(crate) id: String,
    pub(crate) command: InvestigationCommand,
    pub(crate) format: RenderFormat,
}

/// The worker's one message: the shared operation's exit code, its captured
/// output, and — after a successful execution only — the typed replay.
#[derive(Debug)]
pub(crate) struct ReplayDone {
    pub(crate) code: i32,
    pub(crate) text: String,
    pub(crate) replay: Option<Replay>,
}

/// Spawns the replay on a worker thread with its own tokio runtime and
/// returns a non-blocking receiver for its [`ReplayDone`].
pub(crate) fn spawn(runtime: Arc<RuntimeConfig>, task: ReplayTask) -> Receiver<ReplayDone> {
    spawn_with(move |tx| {
        let _ = tx.send(run_replay(&runtime, task));
    })
}

/// The spawn seam tests gate: run a caller-supplied body on the worker
/// thread; the body hands its own [`ReplayDone`] to the channel (or not —
/// that is the detach behaviour being tested).
pub(crate) fn spawn_with<F>(body: F) -> Receiver<ReplayDone>
where
    F: FnOnce(&std::sync::mpsc::Sender<ReplayDone>) + Send + 'static,
{
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || body(&tx));
    rx
}
/// Runs one replay through the shared typed operation, capturing the `emit`
/// output on this worker thread. The shape mirrors the foreground adapter:
/// captured stdout wins unless empty (then stderr); an operation Err is the
/// render/IO failure the adapter would push as an error block, its captured
/// output discarded.
fn run_replay(runtime: &RuntimeConfig, task: ReplayTask) -> ReplayDone {
    crate::commands::capture_output_start();
    let outcome = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(handle) => {
            let state_db = saya_store::SqliteStateStore::new(crate::state_path::state_db_path());
            handle.block_on(run_investigation_outcome(
                task.command,
                runtime,
                task.format,
                // The TUI's stdin answer: it never reads stdin, so a missing
                // secret surfaces as an error rather than a stdin prompt.
                false,
                &state_db,
            ))
        }
        Err(error) => Err(Box::new(error) as Box<dyn std::error::Error>),
    };
    let (out, err) = crate::commands::capture_output_take();
    match outcome {
        Ok(outcome) => {
            let text = if out.trim().is_empty() { err } else { out };
            ReplayDone {
                code: outcome.code,
                text: text.trim_end().to_string(),
                replay: outcome.replay,
            }
        }
        Err(error) => ReplayDone {
            code: 2,
            text: error.to_string(),
            replay: None,
        },
    }
}

/// Applies a finished replay. A success carrying the typed replay renders
/// exactly like the direct-/sql path: one Table block — the box table, the
/// scope line, the evidence line — and never the captured CLI text. The
/// captured text is that same table tab-separated plus the evidence line,
/// both re-derived here from the typed replay (the TUI's slash run passes no
/// `--report`, so nothing else is on stdout), and a System block strips its
/// tabs into concatenated words — the very defect this replaces — so the
/// text is dropped entirely.
///
/// The capture honours the same accounted budget the direct-/sql path uses:
/// `accounted` is the result's verdict against it — production passes
/// [`super::capture::accounted_bytes`]; tests a small-budget walk via
/// `accounted_bytes_within`, the seam [`super::capture::capture_within`]
/// takes. Over budget the result is not captured and one visible system
/// line names the way out; the query succeeded either way, so the replay
/// is still selectable.
///
/// A failure — a non-zero exit or no typed replay — keeps the text block:
/// the operation's output as a system or error block, skipped entirely when
/// it is empty. A typed replay riding a non-zero exit (a successful
/// execution whose later step refused; unreachable from the TUI's slash
/// run) still promotes and captures, as before.
pub(crate) fn complete(
    done: ReplayDone,
    accounted: Option<usize>,
    transcript: &mut Transcript,
    last_query: &mut Option<LastQuery>,
    captured: &mut Option<CapturedResult>,
) {
    match done.replay {
        Some(replay) if done.code == 0 => {
            let table = super::table::with_scope_line(
                super::table::format_table(&replay.result),
                Some(replay.connection.as_str()),
                &replay.result.executed_sql,
            );
            *last_query = Some(LastQuery {
                sql: replay.sql,
                connection: Some(replay.connection),
            });
            transcript.push(
                BlockKind::Table,
                format!("{table}\n{}", replay.evidence.human_line()),
            );
            super::capture::capture_within(
                captured,
                replay.result,
                replay.evidence,
                accounted,
                transcript,
            );
        }
        otherwise => {
            // The output block — unless there is nothing to say: a replay
            // whose captured output and stderr are both empty is no block at
            // all, since an empty block is a transcript glitch, not a message.
            if !done.text.is_empty() {
                if done.code == 0 {
                    transcript.push(BlockKind::System, done.text);
                } else {
                    transcript.push(BlockKind::Error, done.text);
                }
            }
            if let Some(replay) = otherwise {
                *last_query = Some(LastQuery {
                    sql: replay.sql,
                    connection: Some(replay.connection),
                });
                super::capture::capture_within(
                    captured,
                    replay.result,
                    replay.evidence,
                    accounted,
                    transcript,
                );
            }
        }
    }
}

#[cfg(test)]
#[path = "replay_task_tests.rs"]
mod tests;
