//! A helper's `shell`, composed from the job's one permission-mode cell.
//!
//! The root's shell reads the job's cell on every call
//! ([`ShellTool::following_mode`]), so a mode the job states between two calls
//! decides the second. A helper's shell is the same tool over the same cell:
//! a mode stated while the helper runs decides that helper's next call, rather
//! than waiting for the helper to be composed again.
//!
//! What a helper adds is its type's capability, and it is applied *after* the
//! mode: the mode a helper resolves is the narrower of the job's current mode
//! and the capability its type holds. A type that cannot write is composed at
//! `read-only`, so no switch of the job's cell — `bypass` included — can hand
//! the helper a capability it never had.

use saya_agent::{ApprovalPolicy, CancellationToken};
use saya_config::CommandRules;
use saya_harness::runner::HostPolicy;

use crate::agent::tools::consent::Prompt;
use crate::agent::tools::shell_policy::{self, Background, ModeCell, ShellPlacement, ShellTool};

/// Composes one helper's `shell` over `placement` — the helper's own scratch
/// tree over the job's shared working checkout — under the job's mode cell.
/// `None` where this host gives the composition no lane at all, exactly as the
/// root's composition returns none.
pub(crate) fn compose_shell(
    placement: ShellPlacement<'_>,
    mode: &ModeCell,
    capability: ApprovalPolicy,
    prompt: Option<Prompt>,
    host: HostPolicy,
    cancellation: CancellationToken,
    rules: CommandRules,
) -> Option<ShellTool> {
    // A helper's shell lives for the helper, so it owns the background runs it
    // starts: they end when the helper does. The mode it resolves is the job's
    // cell, capped by what this helper's type may ever hold — and the cap
    // decides the composition too, so a read-only type is composed over the
    // sandbox that denies every write, whatever the cell says.
    let capped = mode.capped_at(capability);
    shell_policy::compose(
        placement,
        capped.get(),
        prompt,
        host,
        cancellation,
        Background::Owned,
        rules,
        // A helper's calls are put to the job's operator, so it keeps the host
        // lane its mode allows exactly as the root does.
        shell_policy::HostLane::Allowed,
    )
    .map(|tool| tool.following_mode(capped))
}
