//! `/resume` in a session that is already running: the record it opens, and the
//! two facts that refuse one.
//!
//! The line front's `handle_resume` and the full-screen surface's picker ask the
//! same question — which record does `/resume <id>` open, and may this process
//! open it — so the answer is stated once, here: the current session is saved
//! first, the record is loaded through the store's own resume path
//! ([`resume_session`]), and the folder it pins is read through the one shape
//! decision every run's workspace comes from ([`crate::chat::workspace_shape`]).
//!
//! **One rule for the folder.** A record pins the folder its session worked in.
//! That folder refuses the resume when it is not there any more, and when another
//! live session holds its lease — the same shape decision a run's own composition
//! reads ([`crate::chat::WorkspaceShape`]), asked with the current session as the
//! asker, because the run holding the current session's lease is the one the
//! resume gives up. Each refusal is one sentence naming the record.
//!
//! What the caller does with a loaded record is its own: the line front swaps its
//! session runtime to it (`SessionRuntime::reacquire`), and the full-screen
//! surface opens it as the run it drives (`line_chat::open_surface`).

use std::path::Path;

use saya_store::{RedactedSession, SessionStore};

use super::resume::{SessionDefaults, block_on, state_from_redacted};
use crate::chat::{WorkspaceShape, workspace_shape};
use crate::interactive::session::state::SessionState;

/// How much of a record's first request the surface names it by: one line, cut
/// at a character boundary so a title never splits one.
pub(crate) const TITLE_CHARS: usize = 60;

/// What a `/resume <id>` in a running session loaded, or why it did not.
pub(crate) enum MidResume {
    /// The record the id names, as this process would open it.
    Loaded(Box<SessionState>),
    /// The refusal, in one sentence.
    Refused(String),
}

/// What a mid-session resume is asked: the record store, the session doing the
/// asking, the id, and where this session's job records live — which is where a
/// folder's lease is looked for.
pub(crate) struct ResumeMid<'a> {
    pub(crate) store: &'a saya_store::FsSessionStore,
    pub(crate) current: &'a SessionState,
    pub(crate) id: &'a str,
    pub(crate) state_root: &'a Path,
}

/// The record `id` names, for a session that is already running: the current
/// session is saved first, and the record's folder must be one this process can
/// open. Async, so a surface already on a runtime asks it without a second one;
/// [`resume_mid_session_blocking`] is the same operation for a caller that has
/// none.
pub(crate) async fn resume_mid_session(request: ResumeMid<'_>) -> MidResume {
    let ResumeMid {
        store,
        current,
        id,
        state_root,
    } = request;
    if current.id == id {
        return MidResume::Refused(format!("session {id} is the session already running"));
    }
    // The record left on disk is what the operator comes back to if the swap is
    // refused, so it is written before the one that replaces it is read.
    if let Err(error) = store.save(current.redacted()).await {
        return MidResume::Refused(format!(
            "the current session could not be saved first: {error}"
        ));
    }
    let defaults = SessionDefaults {
        provider: current.provider.clone(),
        model: current.model.clone(),
        allow_data_sharing: current.allow_data_sharing,
        approval_mode: current.approval_mode.clone(),
    };
    match store.load(id).await {
        Ok(Some(value)) => {
            let loaded = state_from_redacted(value, &defaults);
            match openable(&loaded, current, state_root) {
                Ok(()) => MidResume::Loaded(Box::new(loaded)),
                Err(sentence) => MidResume::Refused(sentence),
            }
        }
        // The line front's own words, so a typo reads the same on both surfaces.
        Ok(None) => MidResume::Refused(format!("Session not found: {id}")),
        Err(error) => MidResume::Refused(error.to_string()),
    }
}

/// [`resume_mid_session`] for a caller on no runtime at all — the line front's
/// `/resume` — so both surfaces ask the one operation.
pub(crate) fn resume_mid_session_blocking(request: ResumeMid<'_>) -> MidResume {
    block_on(resume_mid_session(request))
}

/// Why this process may not open `loaded`, or nothing at all.
fn openable(
    loaded: &SessionState,
    current: &SessionState,
    state_root: &Path,
) -> Result<(), String> {
    let Some(folder) = loaded.workspace_root.as_deref() else {
        return Ok(());
    };
    let path = Path::new(folder);
    if !path.is_dir() {
        return Err(format!(
            "cannot resume {}: its folder {folder} is not reachable",
            loaded.id
        ));
    }
    // Asked as the current session, because the run holding its lease is the one
    // the resume gives up: a folder this session leases is free for the record.
    match workspace_shape(path, state_root, &current.id) {
        WorkspaceShape::ReadOnlyFiles { reason } => {
            Err(format!("cannot resume {}: {reason}", loaded.id))
        }
        WorkspaceShape::Writable | WorkspaceShape::NoWorkspace { .. } => Ok(()),
    }
}

/// The words an operator recognises a record by: the first thing they asked it,
/// on one line and cut to [`TITLE_CHARS`]. A record that asked nothing is named
/// by its id.
pub(crate) fn title(record: &RedactedSession) -> String {
    let asked = record
        .turns
        .first()
        .map(|turn| turn.user.clone())
        .or_else(|| {
            record
                .messages
                .iter()
                .find(|message| message.role == "user")
                .map(|message| message.content.clone())
        })
        .unwrap_or_default();
    let one_line = asked.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.is_empty() {
        return record.id.clone();
    }
    match one_line.char_indices().nth(TITLE_CHARS) {
        Some((cut, _)) => format!("{}…", &one_line[..cut]),
        None => one_line,
    }
}

/// The record a running session states, as the one opener reads it: the save
/// drops the compaction summary (the chat session states it), and what a run may
/// carry depends on it.
pub(crate) fn record_of(state: &SessionState) -> RedactedSession {
    let mut record = state.redacted();
    record.compaction_summary = state.compaction_summary.clone();
    record
}

/// What the surface says the resume did: the record's title and the turns it
/// carries.
pub(crate) fn resumed_notice(record: &RedactedSession) -> String {
    let turns = record.turns.len();
    let noun = if turns == 1 { "turn" } else { "turns" };
    format!("resumed {} — {turns} {noun}", title(record))
}
