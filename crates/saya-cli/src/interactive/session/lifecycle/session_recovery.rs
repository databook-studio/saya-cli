//! One session's recovery handle: the checkpoint store, the session id,
//! and the session-scoped enable flag, shared by every turn of one
//! session process. The flag lives in memory only and defaults to OFF —
//! recovery is opt-in (decision 10), nothing flips it in this packet, and
//! it dies with the session. A resumed session re-acquires a whole
//! runtime, so it starts off again; a trust rebind and every recomposition
//! carry the same handle, keeping one flag and one id for the session.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use saya_store::FsSessionStore;

/// The recovery wiring one session carries: the checkpoint store rooted
/// at the sessions root, the session id, and the enable flag every clone
/// shares. Cloned into the universe and handed to the per-turn
/// `DatabaseTools` build as `(store, id, flag)` parts.
#[derive(Clone)]
pub(crate) struct RecoveryHandle {
    store: Arc<FsSessionStore>,
    session_id: String,
    enabled: Arc<AtomicBool>,
}

impl RecoveryHandle {
    /// Builds the handle for one acquired session. No I/O: the store opens
    /// its checkpoint tree lazily at the first capture, and the flag
    /// starts OFF — recovery is opt-in, never enabled at acquisition.
    pub(crate) fn new(sessions_root: &Path, session_id: &str) -> Self {
        Self {
            store: Arc::new(FsSessionStore::new(sessions_root)),
            session_id: session_id.to_owned(),
            enabled: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Whether recovery is on for this session: what `/recovery` reports
    /// and the recorders gate every capture on. The flag cell itself is
    /// live and shared by every clone of the handle.
    pub(crate) fn is_enabled(&self) -> bool {
        use std::sync::atomic::Ordering;
        self.enabled.load(Ordering::Acquire)
    }

    /// Flips the session's recovery flag: what `/recovery on|off` sets.
    /// Every clone of this handle — and every recorder built from `parts`
    /// — sees the same cell, so a toggle reaches the already-attached
    /// recorders on their next edit.
    pub(crate) fn set_enabled(&self, on: bool) {
        use std::sync::atomic::Ordering;
        self.enabled.store(on, Ordering::Release);
    }

    /// The parts the per-turn `DatabaseTools` build attaches: the store,
    /// the session id, and the shared enable flag. Clones share one cell,
    /// so a later toggle reaches the already-attached recorders.
    pub(crate) fn parts(&self) -> (Arc<FsSessionStore>, String, Arc<AtomicBool>) {
        (
            Arc::clone(&self.store),
            self.session_id.clone(),
            Arc::clone(&self.enabled),
        )
    }
}

#[cfg(test)]
#[path = "session_recovery_tests.rs"]
mod tests;
