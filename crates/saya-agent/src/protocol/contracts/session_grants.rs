//! The session's grant store: the tokens the user granted for this session,
//! and the calls each has pre-answered. A sibling of the policy engine rather
//! than part of it — the engine consults the store, the store knows nothing of
//! modes.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// The grammar tokens granted for this session: process lifetime, never
/// persisted, and a resumed session starts empty. Additive only — nothing
/// widens a grant except another explicit user choice.
#[derive(Debug, Clone, Default)]
pub struct SessionGrants {
    tokens: Arc<Mutex<BTreeSet<String>>>,
    /// The calls each token's grant has pre-answered — the count a prompt's
    /// session-history line reads ("3 calls so far"). Incremented by
    /// [`SessionPolicy::resolve`] every time a grant resolves a call to
    /// `Allow`, so the number a prompt states is the store's own count, not
    /// a frontend's tally. Allow-once and denies record nothing here.
    ///
    /// [`SessionPolicy::resolve`]: super::session_policy::SessionPolicy::resolve
    calls: Arc<Mutex<BTreeMap<String, u64>>>,
}

impl SessionGrants {
    /// Records a grant and reports whether it was new, so a first grant can
    /// be journaled exactly once.
    pub fn grant(&self, token: &str) -> bool {
        self.locked().insert(token.to_owned())
    }

    /// Whether this token is granted for the session.
    pub fn is_granted(&self, token: &str) -> bool {
        self.locked().contains(token)
    }

    /// Whether nothing has been granted yet.
    pub fn is_empty(&self) -> bool {
        self.locked().is_empty()
    }

    /// The granted tokens in sorted order — a session listing's words.
    pub fn tokens(&self) -> Vec<String> {
        self.locked().iter().cloned().collect()
    }

    /// The calls the engine allowed under `token` — the session-history
    /// figure a prompt's session line reads. Zero for a token nothing has run
    /// under yet.
    pub fn calls(&self, token: &str) -> u64 {
        self.call_lock().get(token).copied().unwrap_or(0)
    }

    /// Counts one grant-answered call. Internal to the engine: only a
    /// `resolve` deciding `Allow` through a grant increments a token.
    pub(crate) fn record_allowed_call(&self, token: &str) {
        *self.call_lock().entry(token.to_owned()).or_insert(0) += 1;
    }

    /// The grant set is a plain set, so a panicked holder cannot have left it
    /// in a state recovery must fear; taking the guard anyway keeps approving
    /// (and prompting) rather than wedging the session on a poisoned lock.
    fn locked(&self) -> MutexGuard<'_, BTreeSet<String>> {
        self.tokens.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The call counts share the same posture: a poisoned lock keeps
    /// prompting rather than wedging the session.
    fn call_lock(&self) -> MutexGuard<'_, BTreeMap<String, u64>> {
        self.calls.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
