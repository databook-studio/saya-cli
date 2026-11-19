//! The host's one permission-mode cell: who states it at composition, who
//! freezes it, and what a change afterwards is told.
//!
//! The cell itself is [`ModeCell`](crate::agent::tools::shell_policy::ModeCell):
//! the root's `shell` reads it on every call, so the mode stated here is the
//! mode the next call resolves. What this file adds is the surface rule — a
//! detached launch states its approval once and freezes it, because there is
//! no operator an in-run switch could be put to.

use super::CodingApplication;
use crate::agent::tools::shell_policy::{ModeCell, ModeFrozen};
use saya_agent::ApprovalPolicy;
use std::sync::Arc;

/// Who composed a run, and the approval mode it states — the one thing that
/// decides whether that mode stays switchable afterwards.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Launch {
    /// A run with nobody attached to ask: `saya -p`, `saya start`,
    /// `saya follow-up`, and every `saya job` verb — `resume` included. Its
    /// approval is stated once and frozen.
    Detached(ApprovalPolicy),
    /// The TUI job panel and the interactive session: a mode stated here can
    /// still be switched.
    Attached(ApprovalPolicy),
}

impl Launch {
    /// States the mode the job's `shell` reads from the cell. A detached
    /// launch's cell is frozen in the same step, so a later change is refused
    /// rather than applied.
    pub(crate) fn compose(self, application: Arc<CodingApplication>) -> Arc<CodingApplication> {
        match self {
            Self::Detached(policy) => application.with_frozen_approval_policy(policy),
            Self::Attached(policy) => application.with_approval_policy(policy),
        }
    }
}

impl CodingApplication {
    /// States the approval mode the root's `shell` reads. The composition-time
    /// builder: a fresh cell is unfrozen, so the stated mode is what every
    /// holder reads from its next call; a cell already frozen keeps its own,
    /// which [`set_approval_policy`](Self::set_approval_policy) reports.
    pub fn with_approval_policy(self: Arc<Self>, policy: ApprovalPolicy) -> Arc<Self> {
        let _ = self.mode.set(policy);
        self
    }
    /// States a detached composition's mode and freezes it: a headless
    /// `saya -p`, a background `saya start`/`saya follow-up`/`saya job resume`
    /// has no operator a switch could be put to, so nothing moves it after the
    /// start.
    pub(crate) fn with_frozen_approval_policy(
        self: Arc<Self>,
        policy: ApprovalPolicy,
    ) -> Arc<Self> {
        self.mode.freeze_at(policy);
        self
    }
    /// States the mode the root's `shell` resolves from its next call, and
    /// reports a frozen job's refusal rather than assuming the change landed.
    pub fn set_approval_policy(&self, policy: ApprovalPolicy) -> Result<(), ModeFrozen> {
        self.mode.set(policy)
    }
    /// States the mode a freshly reopened host starts with: the same mode this
    /// one holds, frozen again when this one is. A detached start that falls
    /// back to an in-place root reopens through here, so the fallback cannot
    /// hand back a host whose mode a later call could move.
    pub(crate) fn reopen_mode(self: &Arc<Self>, application: Arc<Self>) -> Arc<Self> {
        if self.mode.is_frozen() {
            application.with_frozen_approval_policy(self.mode.get())
        } else {
            application.with_approval_policy(self.mode.get())
        }
    }
}

impl super::super::runtime::NativeCodingJob {
    /// The approval mode the root's `shell` reads when it decides whether a
    /// run outside the sandbox must be put to the operator. Read from the job's
    /// shared cell, so it is the mode stated most recently — not the one the
    /// job was built in.
    pub(crate) fn approval_policy(&self) -> ApprovalPolicy {
        self.application.mode.get()
    }
    /// The job's one permission-mode cell, handed to every policy that decides
    /// a call: the root's shell reads the mode from it on each call, so a mode
    /// stated between two calls is the mode the second one resolves.
    pub(crate) fn approval_mode(&self) -> ModeCell {
        self.application.mode.clone()
    }
}
