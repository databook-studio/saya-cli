//! `/run` action: `--seed-grants` filters the session grants into the child
//! allow-list, then the tail grammar decides between a fresh run panel, run
//! management, an in-shell resume hint, or an error.

use super::super::dispatch_runs;
use super::super::transcript::{BlockKind, Transcript};
use super::outcome::Dispatch;
use crate::config::runtime::RuntimeConfig;
use crate::interactive::session_commands::SessionAction;
use crate::interactive::session_run::{self, RunTail};
use crate::interactive::session_runtime::SessionRuntime;
use crate::render::RenderFormat;

#[allow(clippy::too_many_arguments)]
pub(super) fn apply_run_action(
    action: SessionAction,
    transcript: &mut Transcript,
    runtime: &RuntimeConfig,
    state_db: &saya_store::SqliteStateStore,
    format: RenderFormat,
    session: &mut SessionRuntime,
) -> Option<Dispatch> {
    match action {
        SessionAction::Run(tail) => apply_run(tail, transcript, runtime, state_db, format, session),
        _ => None,
    }
}

fn apply_run(
    tail: String,
    transcript: &mut Transcript,
    runtime: &RuntimeConfig,
    state_db: &saya_store::SqliteStateStore,
    format: RenderFormat,
    session: &mut SessionRuntime,
) -> Option<Dispatch> {
    // The child's own grammar parses the tail in-process (the
    // parser stays the authority); a fresh run drives the
    // run panel as a worker task instead of the nested child
    // a piped session spawns — the alternate screen owns
    // stdout, so the run's observers forward to the panel.
    // `--seed-grants` is the adapter's word: the session's
    // grants are filtered through the run's parser first —
    // accepted ones join the child's `--allow`, refused ones
    // are named before the panel opens.
    let (seed_requested, tail) = session_run::separate_seed_flag(&tail);
    let seed = if seed_requested {
        let seed =
            crate::interactive::session_grants::run_seed(&session.policy().grants().tokens());
        transcript.push(
            BlockKind::System,
            crate::interactive::session_grants::seed_message(&seed),
        );
        Some(seed)
    } else {
        None
    };
    match session_run::parse_run_tail(tail) {
        Ok(RunTail::Start {
            goal,
            mut allow,
            budget,
        }) => {
            if let Some(seed) = seed.as_ref() {
                allow.extend(seed.forwarded.iter().cloned());
            }
            return Some(Dispatch::RunPanel {
                goal,
                allow,
                budget,
            });
        }
        Ok(RunTail::Manage(command)) => {
            dispatch_runs::run_management_command(transcript, runtime, state_db, format, command);
        }
        Ok(RunTail::Resume(run_id)) => transcript.push(
            BlockKind::System,
            format!(
                "Resuming run {run_id} streams to the real terminal, which \
                 the TUI does not own while the panel is up — resume it \
                 from a shell: `saya run resume {run_id}`."
            ),
        ),
        Err(message) => transcript.push(BlockKind::Error, message),
    }
    None
}
