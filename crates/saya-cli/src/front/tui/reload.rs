//! `/reload`, and the apply a closed guided setup runs: one swap, one report.
//!
//! The re-read itself is the crate's ([`crate::config::reload`]): a candidate
//! built from the same launch inputs, its changes classified against the
//! running configuration, and only the copies the operator never overrode
//! re-seeded on the session. What this module adds is the surface's half —
//! putting the candidate in force ([`LiveRuntime::swap`]) and saying the plan's
//! own words. A broken file builds no candidate, so nothing is swapped and the
//! pinned sentence ("The previous configuration is still in use.") is the whole
//! answer.

use std::sync::Arc;

use super::live::LiveRuntime;
use super::ops::Asked;
use crate::config::reload::{ReloadInputs, ReloadOutcome, reload_runtime};
use crate::interactive::session::state::SessionState;

/// Re-reads the configuration from `inputs` and, only when the candidate
/// builds, makes it the one the surface runs on from now. The previous
/// configuration stays in use on every failure, in the reload core's own words
/// — the caller says them; this function never does.
pub(crate) fn apply(inputs: &ReloadInputs, live: &LiveRuntime, state: &mut SessionState) -> Asked {
    // The configuration in force is the comparison's old side, and it is held
    // for this whole call: a reload never compares a candidate against itself.
    let old = live.current();
    match reload_runtime(inputs, old.as_ref(), state) {
        ReloadOutcome::Reloaded { new, text } => {
            live.swap(Arc::from(new));
            Asked::Notice(text)
        }
        ReloadOutcome::Failed(text) => Asked::Error(text),
    }
}
