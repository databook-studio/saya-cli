//! The session's grant surface: the per-turn grant view and the per-family
//! availability decision it feeds.
//!
//! Two halves, one source of truth — the live [`SessionGrants`] store the
//! session's [`SessionPolicy`] holds:
//!
//! - [`GrantView`] is the universe's handle on that store. The session
//!   surfaces (`session_request::run`, the TUI spawn) hold the turn's
//!   policy and bind its store onto the universe before the turn runs —
//!   the same per-turn bind [`crate::grant_token::TurnPrimary`] uses for
//!   the turn's primary connection. Clones of the store share one
//!   `Arc`-backed set, so the view is never a stale copy: a grant recorded
//!   by this turn's decider is visible to the next turn's definitions
//!   through the same set. Unbound — a universe no turn has bound, the run
//!   engine's contained session, the test fixtures that bind nothing — is
//!   the closed shape: no held grant, no grant arm. Fail closed.
//!
//! - [`workspace_files_available`] is the workspace file family's
//!   advertisement decision, consumed by [`super::SessionUniverse::
//!   definitions`]. The family — `workspace_write`, `workspace_edit` — is
//!   advertised exactly where a call can actually run: an approval surface
//!   that can answer the ask (the standing rule), the bypass mode's own
//!   consent, or — new — an explicit `workspace-write` grant held by the
//!   live policy under `ask`, which pre-answers the call the mode would
//!   have asked about (`SessionPolicy::resolve` resolves a granted token
//!   to `Allow` without prompting, so the advertised tools are usable).
//!   The grant widens no other family and no other posture: `read-only`
//!   and `never` deny without consulting grants, `plan` denies before the
//!   approval match, and the chart save rides the approval surface alone.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use saya_agent::{AgentMode, ApprovalPolicy, SessionGrants};

/// The grammar token a session grant for either guarded file tool records
/// (`grant_token.rs`): one token, `workspace-write`, pre-answers both
/// `workspace_write` and `workspace_edit`. The availability check reads the
/// same word the approval engine resolves at the door.
const WORKSPACE_FILE_TOKEN: &str = "workspace-write";

/// The universe's live view of the session's grant store, bound per turn by
/// the surface that holds the policy. `Default` is the unbound, closed
/// shape.
#[derive(Clone, Default)]
pub(crate) struct GrantView(Arc<Mutex<Option<SessionGrants>>>);

impl GrantView {
    /// Binds the turn's live store. Production binds once per turn, from
    /// the policy the surface holds (`SessionPolicy::grants`); the clone is
    /// an `Arc` handle, so the bound view and the deciders read one set.
    pub(crate) fn bind(&self, grants: &SessionGrants) {
        *self.locked() = Some(grants.clone());
    }

    /// The bound store, when a turn bound one.
    pub(crate) fn get(&self) -> Option<SessionGrants> {
        self.locked().clone()
    }

    /// The view is a plain cell, so a panicked holder cannot have left it in
    /// a state the next turn must fear; taking the guard anyway keeps
    /// deciding rather than wedging the session on a poisoned lock — the
    /// engine's own rule for its grant store.
    fn locked(&self) -> MutexGuard<'_, Option<SessionGrants>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Whether the workspace file family (`workspace_write`, `workspace_edit`)
/// may be advertised on this surface. The grant arm requires `ask` — the
/// one mode whose asks a grant pre-answers — and an explicit
/// `workspace-write` token in the live store; the engine's other modes are
/// untouched: `bypass` allows on its own, `read-only` and `never` never
/// consult grants, and `plan` denies before the approval match.
pub(crate) fn workspace_files_available(
    agent_mode: AgentMode,
    mode: ApprovalPolicy,
    can_obtain_approval: bool,
    grants: Option<&SessionGrants>,
) -> bool {
    agent_mode == AgentMode::Build
        && match mode {
            ApprovalPolicy::Ask => {
                can_obtain_approval
                    || grants.is_some_and(|grants| grants.is_granted(WORKSPACE_FILE_TOKEN))
            }
            ApprovalPolicy::Bypass => true,
            ApprovalPolicy::ReadOnly | ApprovalPolicy::Never => false,
        }
}

#[cfg(test)]
#[path = "session_grant_surface_tests.rs"]
mod tests;
