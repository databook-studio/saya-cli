//! The turn's primary connection, bound per turn and read at ask time.

use crate::connection::ConnectionRegistry;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// The turn's primary connection name, shared between the turn and the
/// approval deciders. The deciders are built before the turn's registry
/// exists (`prepare_turn` runs inside the turn), so the session universe
/// holds one of these and the turn binds it from the registry it builds;
/// the suggester reads it at ask time. Bound per turn — a mid-session
/// `/connect` names the next turn's primary, never a stale one. Unbound
/// suggests no token: fail closed, never a guessed name.
#[derive(Clone, Default)]
pub(crate) struct TurnPrimary(Arc<Mutex<Option<String>>>);

impl TurnPrimary {
    /// Binds the registry's primary — its real registry name, or nothing
    /// when no primary resolves.
    pub(crate) fn bind(&self, registry: &ConnectionRegistry) {
        *self.locked() = registry.primary().map(str::to_owned);
    }

    /// The bound primary, when the turn has bound one.
    pub(crate) fn get(&self) -> Option<String> {
        self.locked().clone()
    }

    /// The grant set is a plain cell, so a panicked holder cannot have left
    /// it in a state recovery must fear; taking the guard anyway keeps
    /// suggesting rather than wedging the session on a poisoned lock (the
    /// engine's own rule for its grant store).
    fn locked(&self) -> MutexGuard<'_, Option<String>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
