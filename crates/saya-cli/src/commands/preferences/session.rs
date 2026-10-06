//! The session-surface preference adapter (M04b): `/preferences` on the
//! headless REPL, the TUI, and `--turn-file` run one command through the
//! same shared operations the headless `saya preferences` subcommand uses.
//!
//! The worktree is the SESSION's bound root — the canonical root the
//! session runtime holds, never the launch cwd — and a session with no
//! bound root refuses with the shared [`PreferenceRefusal::NoWorktree`]
//! words. The output is the headless rendering: success exits 0, every
//! refusal exits 2, so every surface says the same words. The adapters map
//! the exit code onto their own shapes (an errored turn on the REPL and
//! `--turn-file`, an Error block in the TUI); nothing here prompts or reads
//! stdin, and a preference never grants a capability.

use super::{PreferenceRefusal, worktree_id_for};
use crate::cli::PreferencesCommand;
use saya_store::SqliteStateStore;
use std::path::Path;

/// Runs one preference command in a session: `root` is the session's bound
/// worktree root (`SessionRuntime::root()`), `None` when nothing binds —
/// which refuses before any operation runs.
pub(crate) async fn run_in_session(
    command: PreferencesCommand,
    root: Option<&Path>,
    state: &SqliteStateStore,
) -> (i32, String) {
    let worktree = match root {
        None => Err(PreferenceRefusal::NoWorktree),
        Some(root) => worktree_id_for(root).map_err(|_| PreferenceRefusal::NoWorktree),
    };
    let outcome = super::execute(command, worktree, state).await;
    super::render::render(&outcome)
}
