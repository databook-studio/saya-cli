//! The remaining composed families' per-family grant availability
//! decisions: scratch, fetch, runner/interpreter, and the host-command
//! lane — consumed by [`super::SessionUniverse::definitions`] beside the
//! workspace file family's own decision (`session_grant_surface`).
//!
//! One decision, four words. A family is advertised exactly where a call
//! can actually run: an approval surface that can answer the ask (the
//! standing rule), the bypass mode's own consent, or — under a headless
//! `ask` — an explicit grant of *that* family held by the live policy,
//! which pre-answers the calls the mode would have asked about
//! (`SessionPolicy::resolve` resolves the call's own token to `Allow`
//! without prompting, so the advertised tools are usable, never the
//! advertised-but-unusable anti-pattern). The family words come from
//! [`crate::grant_token::grant_family`] — the grammar's own prefix
//! grouping, never a re-spelled or re-parsed token: `scratch` is the exact
//! token, `fetch:`/`command:` are prefix families, and `runner` covers
//! both `runner:` and `interpreter:` doors, mirroring `grant_token`'s
//! `runner:<program>`/`interpreter:<program>` suggestions.
//!
//! Availability is not authority. One held token opens one family and no
//! other: a `fetch:` token for one host opens no other host, a
//! `command:` token for one program grants no other — the advertised
//! generic tool is still judged per call against its actual arguments and
//! exact grant, and an uncovered call resolves to `Ask`, which a headless
//! surface denies without prompting. The ungrantable effects
//! (`render_chart`, the fan-out SQL) ride no token and stay on the
//! approval-surface rule alone. `read-only` and `never` never consult
//! grants, and `plan` denies before the approval match — dormant grants
//! under a restrictive posture stay dormant.

use saya_agent::{AgentMode, ApprovalPolicy, SessionGrants};

use crate::grant_token::grant_family;

/// Whether `family` — a [`grant_family`] word, verbatim — is advertised on
/// this surface. The grant arm requires `ask` (the one mode whose asks a
/// grant pre-answers) and Build (a Plan posture denies before the approval
/// match, so nothing grant-shaped advertises there); `bypass` allows on
/// its own; `read-only` and `never` never consult grants. An unbound grant
/// view is the closed shape: no held grant, no grant arm.
fn family_available(
    agent_mode: AgentMode,
    mode: ApprovalPolicy,
    can_obtain_approval: bool,
    grants: Option<&SessionGrants>,
    family: &str,
) -> bool {
    agent_mode == AgentMode::Build
        && match mode {
            ApprovalPolicy::Ask => {
                can_obtain_approval
                    || grants.is_some_and(|grants| {
                        grants
                            .tokens()
                            .iter()
                            .any(|token| grant_family(token) == Some(family))
                    })
            }
            ApprovalPolicy::Bypass => true,
            ApprovalPolicy::ReadOnly | ApprovalPolicy::Never => false,
        }
}

/// The scratch family: a held `scratch` token (`grant_token` suggests it
/// for both `scratch_sql` and `scratch_import`) opens the family. The
/// members' own composition still narrows: `scratch_import` additionally
/// needs a workspace root, the same facts `grant_token` reads before
/// offering the token.
pub(crate) fn scratch_available(
    agent_mode: AgentMode,
    mode: ApprovalPolicy,
    can_obtain_approval: bool,
    grants: Option<&SessionGrants>,
) -> bool {
    family_available(agent_mode, mode, can_obtain_approval, grants, "scratch")
}

/// The fetch family: any held `fetch:<destination>` token opens
/// `http_fetch` — and `http_download` exactly where a workspace root
/// binds (the download needs a destination). One token authorizes one
/// destination: the advertisement opens the family's generic tool, and
/// the call's own destination is still judged per call.
pub(crate) fn fetch_available(
    agent_mode: AgentMode,
    mode: ApprovalPolicy,
    can_obtain_approval: bool,
    grants: Option<&SessionGrants>,
) -> bool {
    family_available(agent_mode, mode, can_obtain_approval, grants, "fetch")
}

/// The runner family: any held `runner:<program>` or
/// `interpreter:<program>` token opens `run_program` — the one tool both
/// doors enter (`grant_family` groups them). A token for a program the
/// composed doors do not carry was refused at `/allow` time
/// (`allow_refusal`), and where no runner composed the family opens
/// nothing.
pub(crate) fn runner_available(
    agent_mode: AgentMode,
    mode: ApprovalPolicy,
    can_obtain_approval: bool,
    grants: Option<&SessionGrants>,
) -> bool {
    family_available(agent_mode, mode, can_obtain_approval, grants, "runner")
}

/// The host-command lane: a held `command:<program>` token opens
/// `run_command` where the lane composed — nowhere without a workspace
/// root, the lane's structural precondition.
pub(crate) fn command_available(
    agent_mode: AgentMode,
    mode: ApprovalPolicy,
    can_obtain_approval: bool,
    grants: Option<&SessionGrants>,
) -> bool {
    family_available(agent_mode, mode, can_obtain_approval, grants, "command")
}

#[cfg(test)]
#[path = "session_grant_families_tests.rs"]
mod tests;
