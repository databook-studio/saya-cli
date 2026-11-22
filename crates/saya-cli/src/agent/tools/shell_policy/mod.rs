//! The one `shell` tool: its advertised schema, its lane decision, and the
//! composition that proves the sandbox it may run in. Every surface that
//! advertises `shell` — the coding root and a `saya run`'s per-step
//! toolsets — builds it here, so what a call costs the operator cannot differ
//! between them.
//!
//! The lane decision is the whole of the policy, and it is resolved once
//! ([`ShellPolicy::lane`]) and read by both `approve` and `execute`, so the
//! two can never disagree about where a call runs or why it was refused:
//!
//! - **Host** — `sandbox` omitted or false, and any call at all on a host
//!   where no sandbox could be proven. This is the default lane: the command
//!   runs on this machine. `bypass` allows it outright, `ask` puts it to the
//!   operator, and `read-only` serves it from the sandbox instead. A host
//!   child inherits this process's environment minus the secrets
//!   [`host_policy`](super::coding::shell_secrets::host_policy) names.
//! - **Sandboxed** — `sandbox: true`, on a host where the sandbox proved. The
//!   call runs with no prompt in every mode but `never`: the confinement is
//!   the consent. Under `read-only` the root is write-denied, so the command
//!   can read and run but change nothing.
//!
//! Two fallbacks keep a host call from ever running unasked. A host call under
//! `ask` with no operator to answer it — a headless run — runs in the sandbox
//! when one is proven, carrying in its result the reason it was rerouted
//! ([`SANDBOX_DOWNGRADE_NOTICE`]), and is refused with a reason naming the flag
//! that would reach the host when none is; and under `read-only` a host call is
//! served from the sandbox, the only lane that mode keeps.
//!
//! The confinement itself is the harness [`WorkspaceShell`](saya_harness::runner::WorkspaceShell): `/bin/sh -c`
//! under the proven spawn, empty-then-declared environment, bounded and
//! redacted capture, process-group kill. This module adds no second spawn path.
//!
//! # Who is asking
//!
//! A helper's shell is built here too: the same policy over the
//! shared working checkout W, with the helper's own scratch `HOME`/`TMPDIR`
//! ([`ShellPlacement::helper`]) and a subject line that names the helper
//! ([`Caller::Helper`](crate::agent::tools::consent::Caller)). A [`prompt`]
//! pairs the operator with that line, so the panel — which renders the line as
//! the prompt's first — answers for the root and for a helper alike without
//! either surface wording its own prompt. A job wraps its operator in
//! [`SerialisedOperator`](crate::agent::tools::consent::SerialisedOperator)
//! once, so several helpers asking at once queue: one prompt is pending at a
//! time, and a waiting ask still honours cancellation.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use saya_agent::{ApprovalPolicy, SessionPolicy};

/// The host-lane policy a `sandbox: false` call runs under: the environment
/// variable names and secret values a host child must not inherit. Built where
/// the resolved config and the values saya already holds are in hand
/// ([`host_policy`](super::coding::shell_secrets::host_policy)), and reached
/// through this module so every surface that composes a shell shares one
/// policy.
pub(crate) use super::coding::shell_secrets::host_policy;

mod arguments;
mod compose;
mod model_output;
mod placement;
mod policy;
mod poll;
mod prompt;
mod sanitise;
mod schema;
mod tool;
mod view;

#[cfg(test)]
#[path = "concurrency_tests.rs"]
mod concurrency_tests;

#[cfg(test)]
#[path = "mode_cell_tests.rs"]
mod mode_cell_tests;

#[cfg(test)]
#[path = "policy_grant_tests.rs"]
mod policy_grant_tests;

#[cfg(test)]
#[path = "policy_tests.rs"]
mod policy_tests;

#[cfg(test)]
#[path = "schema_tests.rs"]
mod schema_tests;

#[cfg(test)]
#[path = "spool_read_tests.rs"]
mod spool_read_tests;

/// The shared permission-mode cell: one per job, handed by handle to every
/// policy that decides a call, so the mode one surface states is the mode the
/// next call resolves — no policy keeps a copy that can go stale. It is the
/// session's own cell shape, and a frozen one refuses every change: a headless
/// or background run states its approval before it starts.
///
/// A cell can also be **capped** ([`ModeCell::capped_at`]): the same cell read
/// through the widest mode one caller's capability allows. That is how a helper
/// follows the job's mode and still cannot exceed its own type — the cap is
/// applied after the mode, never instead of it.
#[derive(Debug, Clone)]
pub(crate) struct ModeCell {
    mode: SessionPolicy,
    frozen: Arc<AtomicBool>,
    /// The widest mode this view may resolve. `None` is a cell that states its
    /// mode whole, which is every cell a surface composes for itself.
    ceiling: Option<ApprovalPolicy>,
}

/// The refusal a frozen cell gives, so a caller reports a change that did not
/// land rather than assuming it did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the permission mode is frozen: this run's approval is stated before it starts")]
pub struct ModeFrozen;

impl ModeCell {
    /// A cell of its own at `mode`: the mode of a surface that composes it
    /// once and never states it again.
    pub(crate) fn new(mode: ApprovalPolicy) -> Self {
        Self {
            mode: SessionPolicy::new(mode),
            frozen: Arc::new(AtomicBool::new(false)),
            ceiling: None,
        }
    }

    /// A view of this cell that never resolves wider than `ceiling`: the
    /// narrowing a capability applies *after* the mode is read. The view shares
    /// this cell's mode, so a switch is still read at the next call; a mode the
    /// cap already covers is resolved as the cell states it, never widened.
    pub(crate) fn capped_at(&self, ceiling: ApprovalPolicy) -> Self {
        Self {
            mode: self.mode.clone(),
            frozen: Arc::clone(&self.frozen),
            ceiling: Some(ceiling),
        }
    }

    /// The mode the next call resolves: the cell's own, or — where a caller
    /// capped it — the narrower of that and the cap.
    pub(crate) fn get(&self) -> ApprovalPolicy {
        match self.ceiling {
            Some(ceiling) => narrower(self.mode.mode(), ceiling),
            None => self.mode.mode(),
        }
    }

    /// States the mode, refused on a frozen cell.
    pub(crate) fn set(&self, mode: ApprovalPolicy) -> Result<(), ModeFrozen> {
        if self.frozen.load(Ordering::Relaxed) {
            return Err(ModeFrozen);
        }
        self.mode.set_mode(mode);
        Ok(())
    }

    /// Whether this cell refuses every change: the composition that opened it
    /// stated its approval rather than leaving it switchable.
    pub(crate) fn is_frozen(&self) -> bool {
        self.frozen.load(Ordering::Relaxed)
    }

    /// States the mode and freezes it in one step: a composition whose approval
    /// is stated, not switched.
    pub(crate) fn freeze_at(&self, mode: ApprovalPolicy) {
        self.mode.set_mode(mode);
        self.frozen.store(true, Ordering::Relaxed);
    }
}

/// The narrower of two modes: the order a capability and the job's current mode
/// combine under. `never` holds no tool at all, so it is narrowest;
/// `read-only` reaches nothing outside the sandbox; the two asking modes put a
/// host call to the operator, and are one width because the shell lane decides
/// them alike; and `bypass` allows the host outright, so it is widest.
fn narrower(mode: ApprovalPolicy, ceiling: ApprovalPolicy) -> ApprovalPolicy {
    if width(mode) <= width(ceiling) {
        mode
    } else {
        ceiling
    }
}

fn width(mode: ApprovalPolicy) -> u8 {
    match mode {
        ApprovalPolicy::Never => 0,
        ApprovalPolicy::ReadOnly => 1,
        ApprovalPolicy::Ask | ApprovalPolicy::AcceptEdits => 2,
        ApprovalPolicy::Bypass => 3,
    }
}

pub(crate) use compose::{Background, HostLane, advertised, compose};
// The sandbox and host-lane constructors are read only from the
// platform-gated shell tests (the deleted session shell was their last
// production caller), so they are re-exported for those tests alone.
#[cfg(test)]
pub(crate) use compose::{compose_host_only, compose_sandboxed, prove};
pub(crate) use placement::ShellPlacement;
pub(crate) use poll::WallClock;
// The refusal and notice texts are read only from test code and `ShellPolicy`
// only from the doc link in `interactive::session::universe::shell`, so none is
// a live path in a non-test build; callers still name them through this module.
#[allow(unused_imports)]
pub(crate) use policy::{HOST_APPROVAL_UNAVAILABLE, SANDBOX_DOWNGRADE_NOTICE, ShellPolicy};
pub(crate) use prompt::prompt;
// The prompt's line is read only from test code, which asserts the wording the
// operator is shown; callers still name it through this module.
#[cfg(test)]
pub(crate) use arguments::validate as validate_arguments;
pub(crate) use model_output::spool_dir;
#[allow(unused_imports)]
pub(crate) use prompt::line;
pub(crate) use sanitise::sanitise_prompt_text;
pub(crate) use schema::{
    BackgroundOwner, MAX_COMMAND_BYTES, MAX_TIMEOUT_MILLIS, Operation, background_definitions,
    definition, is_shell_name, operation, reaches_background, starts_background,
};
pub(crate) use tool::ShellTool;
