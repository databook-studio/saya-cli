//! The configuration the surface's runtime is on, and the swap `/reload` makes.
//!
//! The launch resolves one configuration and the surface's commands read it as
//! they run: a query builds its pool from it, a read answers from it. A reload
//! must therefore move what those reads see *next*, without disturbing the one
//! a query already running is holding — so the handle is a shared cell, and a
//! reader takes an [`Arc`] clone before it starts. Nothing here builds a
//! configuration or decides whether it is valid: that is
//! [`super::reload`]'s, over the crate's one reload core.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::config::runtime::RuntimeConfig;

/// The configuration in force, swappable between turns.
pub(crate) struct LiveRuntime {
    current: Mutex<Arc<RuntimeConfig>>,
}

impl LiveRuntime {
    /// A cell over the configuration the launch resolved.
    pub(crate) fn new(runtime: Arc<RuntimeConfig>) -> Self {
        Self {
            current: Mutex::new(runtime),
        }
    }

    /// The configuration a read that starts now runs on. The clone keeps that
    /// configuration alive for the whole read even if a reload swaps in the
    /// middle of it, so a query never changes configuration halfway.
    pub(crate) fn current(&self) -> Arc<RuntimeConfig> {
        Arc::clone(&self.lock())
    }

    /// Puts a built candidate in force. Only a candidate that resolved reaches
    /// here: a failed reload never calls this, which is what keeps the previous
    /// configuration in use.
    pub(crate) fn swap(&self, next: Arc<RuntimeConfig>) {
        *self.lock() = next;
    }

    /// The cell, with a poisoned lock read as the value it last held: a
    /// panicking reader must not take the surface's configuration with it.
    fn lock(&self) -> MutexGuard<'_, Arc<RuntimeConfig>> {
        self.current.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
