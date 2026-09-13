//! The run boundary's mode admission: the type every entry point into a run
//! takes, and the one constructor a bypass policy cannot pass.
//!
//! A run's approval is its `--allow` scopes — typed, per-capability,
//! journaled. Bypass is a session mode: a blanket per-call consent, and a
//! run has no per-call consent to replace. The start guard taught the rule;
//! the resume entry never consulted it (U6 defect 1: a resumed run composed
//! a frozen policy in bypass mode and auto-allowed every ask-shaped call),
//! so the rule became a property of the type instead of a property of
//! remembering: every entry into a run — the headless fresh run, the host
//! panel's fresh run, the resume, any future one — must hold a
//! [`RunApproval`], and [`RunApproval::admit`] is the only way to build one.
//! The run's composition (`assembly::assemble`, the drive, the engine
//! resume) takes the type, never a raw mode, so a third entry point added
//! later cannot reach the machinery without consulting the guard.

use saya_agent::ApprovalPolicy;

/// A run's admitted approval mode: any session mode but bypass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RunApproval(ApprovalPolicy);

impl RunApproval {
    /// Admits a session mode into a run. Refused before anything exists on
    /// disk: a run's approval is its `--allow` scopes, and bypass is a
    /// session mode's blanket per-call consent — a run has no per-call ask
    /// to replace, so a bypass run would silently auto-allow every
    /// ask-shaped call its seeds do not name. The refusal names both facts
    /// (DESIGN §6, test 16).
    pub(crate) fn admit(approval: ApprovalPolicy) -> Result<Self, Box<dyn std::error::Error>> {
        if approval == ApprovalPolicy::Bypass {
            return Err(
                "a run's approval is its `--allow` scopes; bypass is a session mode".into(),
            );
        }
        Ok(Self(approval))
    }

    /// The admitted session mode — the frozen decider's mode, once the run's
    /// composition is reached. Bypass is unreachable here by construction.
    pub(crate) fn policy(self) -> ApprovalPolicy {
        self.0
    }
}
