//! Shared project-preference operations and the `saya preferences`
//! subcommand (M04a).
//!
//! One bounded CRUD family over explicit project preferences —
//! user-confirmed text guidance scoped to the current worktree — needing no
//! database profile and clearly separate from `/remember`/`/forget`/
//! `contracts` (typed database knowledge). The operations return typed
//! outcomes the slash packet (M04b) reuses, and [`worktree_id_for`] is the
//! single worktree-identity derivation those packets share: the root is the
//! canonical worktree top, hashed once, never the launch cwd.

pub(crate) mod operations;
pub(crate) mod render;
#[cfg(test)]
#[path = "operations_tests.rs"]
mod tests;

use crate::cli::PreferencesCommand;
use crate::commands::output::{failure_message, result};
use crate::interactive::session_workspace::resolve_root;
use crate::render::RenderFormat;
use saya_store::SqliteStateStore;
use saya_types::{ProjectPreference, WorktreeId};
use std::path::Path;

/// The typed outcome of one preference operation; every variant is
/// renderable by [`render::render`] and reusable by the slash surface.
/// These are the packet's shared contract: M04b (slash) and M03 (recall)
/// import them from here rather than re-parsing rendered output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PreferenceOutcome {
    Added(ProjectPreference),
    Listed {
        worktree_label: String,
        items: Vec<ProjectPreference>,
    },
    Edited(ProjectPreference),
    Forgotten {
        existed: bool,
        id: String,
    },
    ForgottenAll(usize),
    Refused(PreferenceRefusal),
}

/// Why an operation refused. Every variant is a one-sentence remedy in the
/// renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PreferenceRefusal {
    NoWorktree,
    TextNotAccepted,
    TooMany,
    TooMuchText,
    NotFound,
    InvalidId,
    Unavailable,
}

/// The worktree identity for an already-resolved canonical root: the digest
/// the store scopes rows by. M03 and M04b reuse this — never a second
/// derivation. An unusable root (practically: empty or NUL-carrying) is an
/// error string, not a silently different identity.
pub(crate) fn worktree_id_for(root: &Path) -> Result<WorktreeId, String> {
    WorktreeId::from_canonical_path(&root.to_string_lossy()).map_err(|error| error.to_string())
}

/// This subcommand's worktree: the launch directory resolves through the
/// same `resolve_root` a session binds (app.rs already refuses
/// `--workspace` on a subcommand, so nothing explicit reaches here), and a
/// run outside any git worktree is the [`PreferenceRefusal::NoWorktree`]
/// refusal — no root binds, nothing is stored.
fn resolved_worktree() -> Result<WorktreeId, PreferenceRefusal> {
    let cwd = std::env::current_dir().map_err(|_| PreferenceRefusal::NoWorktree)?;
    match resolve_root(None, &cwd) {
        Ok(Some(root)) => worktree_id_for(&root).map_err(|_| PreferenceRefusal::NoWorktree),
        Ok(None) | Err(_) => Err(PreferenceRefusal::NoWorktree),
    }
}

/// Runs one preferences subcommand against the on-disk state store. Never
/// prompts and never reads stdin.
pub(crate) async fn run(
    command: PreferencesCommand,
    format: RenderFormat,
    state: &SqliteStateStore,
) -> Result<i32, Box<dyn std::error::Error>> {
    let outcome = match command {
        PreferencesCommand::List => list(state).await,
        PreferencesCommand::Add { text } => add(state, &text).await,
        PreferencesCommand::Edit { id, text } => edit(state, &id, &text).await,
        PreferencesCommand::Forget { id } => forget(state, &id).await,
        PreferencesCommand::ForgetAll => forget_all(state).await,
    };
    let (code, message) = render::render(&outcome);
    if code == 0 {
        result(message, format)
    } else {
        failure_message(code, message, format)
    }
}

async fn list(state: &SqliteStateStore) -> PreferenceOutcome {
    match resolved_worktree() {
        Ok(worktree) => operations::list_preferences(state, &worktree).await,
        Err(refusal) => PreferenceOutcome::Refused(refusal),
    }
}

async fn add(state: &SqliteStateStore, text: &str) -> PreferenceOutcome {
    match resolved_worktree() {
        Ok(worktree) => operations::add_preference(state, &worktree, text).await,
        Err(refusal) => PreferenceOutcome::Refused(refusal),
    }
}

async fn edit(state: &SqliteStateStore, id: &str, text: &str) -> PreferenceOutcome {
    match resolved_worktree() {
        Ok(worktree) => operations::edit_preference(state, &worktree, id, text).await,
        Err(refusal) => PreferenceOutcome::Refused(refusal),
    }
}

async fn forget(state: &SqliteStateStore, id: &str) -> PreferenceOutcome {
    match resolved_worktree() {
        Ok(worktree) => operations::forget_preference(state, &worktree, id).await,
        Err(refusal) => PreferenceOutcome::Refused(refusal),
    }
}

async fn forget_all(state: &SqliteStateStore) -> PreferenceOutcome {
    match resolved_worktree() {
        Ok(worktree) => operations::forget_all_preferences(state, &worktree).await,
        Err(refusal) => PreferenceOutcome::Refused(refusal),
    }
}
