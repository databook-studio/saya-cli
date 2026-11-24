//! The running job a TUI switch moves: what the switch states on the job's one
//! cell, and the line its answer adds, so the user is told which of the two
//! happened.
//!
//! The session's [`switch`](super::switch) owns the decision; this is its job
//! adapter, called after every one of the switch's own checks, so a mode the
//! session refuses is never stated on the job either. Stating it never widens a
//! job's own posture beyond what the user's choice says: `plan` carries no
//! consent posture of its own, so it is stated as `read-only` — the posture its
//! own words name — and never over a job already at `never`, the one posture
//! narrower than that.

use saya_agent::ApprovalPolicy;

use super::FROZEN_REFUSAL;
use crate::agent::tools::shell_policy::ModeCell;
use crate::interactive::session::commands::approval_name;

/// The line the switch's answer adds for `job`, after stating the mode the
/// switch settled on. `consent` is the switch's own consent posture — `None`
/// for a mode that states none, which is `plan`.
pub(super) fn line(job: &ModeCell, consent: Option<ApprovalPolicy>) -> String {
    match posture(consent, job.get()) {
        Some(mode) => match job.set(mode) {
            Ok(()) => format!(
                "The running job switched too: its next call reads {}.",
                approval_name(mode)
            ),
            // A TUI-started job is switchable, so this defends a composition
            // that froze the cell anyway: the session moved, the job did not,
            // and it is said in the session's own frozen words.
            Err(_) => format!("The running job keeps its mode: {FROZEN_REFUSAL}."),
        },
        None => format!(
            "The running job keeps its mode: {} is narrower than read-only, and a switch never widens it.",
            approval_name(job.get())
        ),
    }
}

/// The posture a switch states on the job's cell, or `None` where it may state
/// none: the mode's own consent posture, and `plan`'s `read-only` — never over
/// a job already narrower than that.
pub(crate) fn posture(
    consent: Option<ApprovalPolicy>,
    current: ApprovalPolicy,
) -> Option<ApprovalPolicy> {
    match consent {
        Some(mode) => Some(mode),
        None if current == ApprovalPolicy::Never => None,
        None => Some(ApprovalPolicy::ReadOnly),
    }
}
