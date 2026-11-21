//! The facts a one-turn composition states about itself.
//!
//! `saya ask` — and each of its candidate attempts — is one message, one turn,
//! one answer. Two of that surface's own facts follow from it, and both are
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

use std::path::PathBuf;

use super::LineSession;
use crate::commands::coding::root_mode::session_state_root;

/// Whether the composition is a surface's one-shot question: one message, one
/// turn, one answer. A one-shot run inherits no continuation context, because
/// there is nothing before its first turn for it to inherit.
#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum OneShot {
    Yes,
    No,
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
