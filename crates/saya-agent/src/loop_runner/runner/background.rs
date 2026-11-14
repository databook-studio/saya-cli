//! Background shell completions: the one short notice the loop appends after
//! a run ends, and the wait a headless run makes instead of asking the model
//! again. The notice carries no command output — the model reads that with
//! `shell_output` — and several exits in one turn coalesce into one block.

use std::time::{Duration, Instant};

use crate::{AgentError, ToolExecutor};

use super::RunState;

/// One coalesced notice for the runs that ended since the last request, or
/// `None` when none did. Each exit is one line; the lines travel as one user
/// message after the turn's tool results.
pub(super) fn notice(tools: &dyn ToolExecutor) -> Option<String> {
    let exits = tools.take_background_exits();
    if exits.is_empty() {
        return None;
    }
    let lines: Vec<String> = exits
        .into_iter()
        .map(|exit| {
            let ended = if exit.killed {
                format!("was killed after {} s", exit.elapsed.as_secs())
            } else if let Some(code) = exit.exit_code {
                format!("exited with code {code} after {} s", exit.elapsed.as_secs())
            } else {
                format!("exited after {} s", exit.elapsed.as_secs())
            };
            format!(
                "background shell {} {ended} ({} output lines; read with shell_output {})",
                exit.id, exit.output_lines, exit.id
            )
        })
        .collect();
    Some(lines.join("\n"))
}

/// Waits for the next background exit on a headless run. Each wait is one
/// slice, bounded by the job's remaining wall clock; when the clock is spent
/// (or absent) the caller finishes the run as it always did. Returns whether
/// a notice is ready.
pub(in crate::loop_runner) async fn wait_for_exit(
    state: &RunState<'_>,
    slice: Duration,
) -> Result<bool, AgentError> {
    if slice.is_zero() {
        return Ok(false);
    }
    loop {
        if state.tools.background_notice_pending() {
            return Ok(true);
        }
        if !state.tools.background_live() {
            return Ok(false);
        }
        let remaining = state
            .progress
            .as_ref()
            .and_then(|progress| progress.remaining_wall());
        if remaining.is_some_and(|remaining| remaining.is_zero()) {
            return Ok(false);
        }
        let now = Instant::now();
        let deadline = match remaining {
            Some(remaining) => (now + slice).min(now + remaining),
            None => now + slice,
        };
        if state
            .tools
            .wait_background_exit(deadline, &state.cancellation)
            .await
        {
            return Ok(true);
        }
        if state.cancellation.is_cancelled() {
            // A budget cancellation is the progress guard ending the job; the
            // no-tool path below reports it. Any other cancellation is an
            // error, exactly as the ordinary path treats it.
            if !matches!(
                state.cancellation.reason(),
                Some(crate::CancellationReason::Budget(_))
            ) {
                super::super::check_cancelled(&state.cancellation)?;
            }
            return Ok(false);
        }
        // With no wall clock there is one bounded slice, not an unbounded
        // wait; with one, the loop re-arms until the clock is spent.
        if remaining.is_none() {
            return Ok(false);
        }
    }
}
