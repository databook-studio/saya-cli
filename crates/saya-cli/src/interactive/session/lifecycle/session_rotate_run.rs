//! `/rotate`'s one runner: the steps both surfaces share. Saves the parent
//! record, builds the pure successor, publishes it through the store
//! (lineage only), and re-acquires the runtime for the successor id —
//! swapping state and engine side only when every step succeeded. A refusal
//! changes nothing; a failed re-acquire leaves the successor on disk and
//! resumable, with the current session kept and held.

use saya_agent::ApprovalPolicy;
use saya_store::{FsSessionStore, SessionStore, StoreError};

use super::session_rotate::rotate_state;
use crate::config::runtime::RuntimeConfig;
use crate::interactive::session::lifecycle::resume::block_on;
use crate::interactive::session::runtime::SessionRuntime;
use crate::interactive::session::state::SessionState;

/// The outcome of `/rotate`.
#[derive(Debug)]
pub(crate) enum RotateResult {
    /// The successor is published and live: state and engine side swapped.
    Rotated {
        /// The parent session's id.
        old: String,
        /// The successor session's id.
        new: String,
    },
    /// Nothing changed: no successor was published and the current session
    /// stands exactly as it was.
    Refused(String),
    /// The successor was published but the swap failed: the current session
    /// is kept and still held, and the successor is on disk, resumable.
    Incomplete(String),
}

/// The over-cap refusal, the same words wherever the cap is measured: a
/// successor that still exceeds the 4 MiB record cap is a refusal, not a
/// silent oversized write.
const OVER_CAP: &str =
    "The successor would still exceed the 4 MiB session limit; compact further before rotating.";

/// The success sentence, identical on every surface.
pub(crate) fn rotated_message(old: &str, new: &str) -> String {
    format!(
        "Rotated into a new session {new} (a successor of {old}). Your summary, notes and \
         tail of the conversation carried over; grants, approvals and the recovery setting \
         did not. Resume the earlier session any time with /resume {old}."
    )
}

/// Runs the rotation with the successor id this process derives.
pub(crate) fn rotate_session(
    store: &FsSessionStore,
    runtime: &RuntimeConfig,
    state: &mut SessionState,
    session: &mut SessionRuntime,
) -> RotateResult {
    let successor_id = crate::interactive::session::lifecycle::resume::new_id();
    rotate_session_with_id(store, runtime, state, session, &successor_id)
}

/// [`rotate_session`] with the successor id injected, so the swap and its
/// refusal are testable against a known id. The rotation id rides the
/// successor id: a store-shaped id (≤ 64 characters, `[A-Za-z0-9_-]`),
/// which the `<UTC stamp>-<nanos hex>` shape qualifies as.
pub(crate) fn rotate_session_with_id(
    store: &FsSessionStore,
    runtime: &RuntimeConfig,
    state: &mut SessionState,
    session: &mut SessionRuntime,
    successor_id: &str,
) -> RotateResult {
    let old = state.id.clone();
    // (1) The parent record exists and is current before anything
    // publishes: `publish_successor` requires the parent to exist and
    // load. A parent already over the cap refuses with the same
    // compact-further words — the successor cannot be smaller than the
    // session it carries.
    // (1) The parent record exists and is current before anything
    // publishes: `publish_successor` requires the parent to exist and
    // load. A parent already over the cap refuses with the same
    // compact-further words — the successor cannot be smaller than the
    // session it carries.
    if let Err(error) = block_on(store.save(state.redacted())) {
        return RotateResult::Refused(match error {
            StoreError::LimitExceeded => OVER_CAP.to_owned(),
            other => other.to_string(),
        });
    }
    // (2)+(3) The pure successor. A refusal here is checked before anything
    // publishes: the session stands, no successor record exists.
    let successor = match rotate_state(state, successor_id, successor_id) {
        Ok(successor) => successor,
        Err(refusal) => return RotateResult::Refused(refusal.sentence().to_owned()),
    };
    // (4) Publish: lineage only; the store writes the parent link itself.
    if let Err(error) = store.publish_successor(&old, successor_id, successor.redacted()) {
        return RotateResult::Refused(match error {
            StoreError::LimitExceeded => OVER_CAP.to_owned(),
            other => format!("Rotation was not completed: {other}"),
        });
    }
    // (5) Re-acquire for the successor id: a failed swap keeps the current
    // session held and names the resumable successor.
    if let Err(error) = session.reacquire(
        runtime,
        successor.workspace_root.as_deref(),
        successor_id,
        successor
            .approval_mode
            .parse()
            .unwrap_or(ApprovalPolicy::Ask),
    ) {
        return RotateResult::Incomplete(format!(
            "Rotation was not completed: {error}. The successor was saved as {successor_id}; \
             resume it with /resume {successor_id} after fixing the cause."
        ));
    }
    // (6) The swap: the same lines `/resume` runs. The header facts ride the
    // NEW universe: `reacquire` recomposed it for the successor, and the
    // successor state arrived carrying the parent's runtime-only values.
    *state = successor;
    state.bind_runtime_endpoint(runtime.resolved.ai.base_url.as_deref());
    session.universe().seed_tasks(state.task_list.clone());
    state.sync_host_posture(&session.universe());
    RotateResult::Rotated {
        old,
        new: successor_id.to_owned(),
    }
}

#[cfg(test)]
#[path = "session_rotate_run_tests.rs"]
mod session_rotate_run_tests;
