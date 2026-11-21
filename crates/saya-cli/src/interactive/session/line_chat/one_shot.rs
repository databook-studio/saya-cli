//! The facts a one-turn composition states about itself.
//!
//! `saya ask` — and each of its candidate attempts — is one message, one turn,
//! one answer. Four of that surface's own facts follow from it, and all are
//! stated here rather than in the composition that consumes them:
//!
//! - **It records nothing in the folder it asks about.** A question asked in a
//!   folder the configured coding state root sits inside is still a question: it
//!   records under a private root of its own there instead of being refused
//!   before it ever reaches its provider.
//! - **It inherits nothing.** The continuation block states what a *continuing*
//!   run already knows, and it carries that run's own identity — its snapshot,
//!   its plan digest, the candidate it readies. A run whose job began a moment
//!   ago has nothing before its first turn, so it states none of it, and two
//!   identical questions send the provider identical request bytes.
//! - **It plans no outcome view.** The advisory readiness precompute fills the
//!   outcome a finishing job records, and a question records none: its scan
//!   would be started on the turn's first text and still running when the
//!   surface closed the run.
//! - **It needs no workspace to be asked in.** A folder that cannot root an
//!   in-place job is not a place a job can be rooted, but it is still a place a
//!   question can be asked: the question runs there without a workspace — the
//!   database tools it resolved and nothing else — instead of being refused.

use std::path::PathBuf;
use std::sync::Arc;

use super::LineSession;
use crate::chat::WorkspaceShape;
use crate::commands::coding::{CodingApplication, root_mode::session_state_root};

/// Whether the composition is a surface's one-shot question: one message, one
/// turn, one answer. A one-shot run inherits no continuation context, because
/// there is nothing before its first turn for it to inherit.
#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum OneShot {
    Yes,
    No,
}

/// The workspace a one-turn ask states: the folder roots the run when it can
/// root an in-place job, and there is no workspace at all when it cannot — the
/// home directory, the filesystem root, saya's own state directory.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Workspace {
    /// The folder roots the run: its files are readable and searchable, and a
    /// write path is the folder's own `[tools] allow` decision.
    Here,
    /// The folder cannot root an in-place job, so the question runs with no
    /// workspace: the database tools it resolved are its whole surface, and no
    /// file, shell or write tool is offered.
    None,
}

/// The state root a one-turn ask records under: `None` when the configured one
/// is already outside the folder it asks about, and a private root of its own
/// when it is not.
pub(super) fn state_root(parts: &LineSession) -> Result<Option<PathBuf>, String> {
    let configured = session_state_root(&parts.runtime).map_err(|error| error.to_string())?;
    let folder = std::fs::canonicalize(&parts.folder).unwrap_or_else(|_| parts.folder.clone());
    if !configured.starts_with(&folder) {
        return Ok(None);
    }
    Ok(Some(
        std::env::temp_dir().join(format!("saya-ask-{}", parts.session)),
    ))
}

/// The workspace a one-turn ask has, read from the folder it was asked in: a
/// shape that states no workspace is a folder no in-place job can be rooted in,
/// and the question is asked there without one rather than refused.
pub(super) fn workspace(shape: &WorkspaceShape) -> Workspace {
    match shape {
        WorkspaceShape::NoWorkspace { .. } => Workspace::None,
        WorkspaceShape::Writable | WorkspaceShape::ReadOnlyFiles { .. } => Workspace::Here,
    }
}

/// The facts a one-turn run states on the host it is composed over: no
/// continuation to inherit, no outcome view to fill.
pub(super) fn state_on(application: &Arc<CodingApplication>) {
    application.with_continuation_context(false);
    application.with_one_turn();
}

#[cfg(test)]
#[path = "one_shot_tests.rs"]
mod tests;
