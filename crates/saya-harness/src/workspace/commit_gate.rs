//! The commit gate for workspace patch publication: one process-wide mutex
//! that every Saya patch writer holds across its fresh-read → compare →
//! publish window, so two Saya writers cannot interleave their stale-version
//! checks.
//!
//! This is an optimistic stale-version guard among Saya's own writers, not an
//! OS-level compare-and-swap: it serializes writers that cooperate by process
//! design, and it does not promise protection from a non-cooperating external
//! program racing the final check.

use std::sync::{Mutex, MutexGuard};

static COMMIT_GATE: Mutex<()> = Mutex::new(());

/// Holds the commit gate until the returned guard is dropped. Recovers from a
/// poisoned lock: a writer that panicked mid-commit must not wedge every
/// later writer.
pub(super) fn hold() -> MutexGuard<'static, ()> {
    COMMIT_GATE
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}
