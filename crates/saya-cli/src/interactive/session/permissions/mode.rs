//! The session's permission-mode switch: one operation, three adapters.
//! `/mode`, the keyboard cycle, and the launch all move the session through
//! this module, so the pair the session runs under has one author and the
//! refusal for `bypass` has one wording.
//!
//! A switch is never a second policy path. It sets the pair the engine
//! already reads — the consent posture on the session's one
//! [`SessionPolicy`] and the task posture beside it — and every call still
//! resolves through `SessionPolicy::resolve`. The mode cells are shared, so
//! a clone already handed to an in-flight turn's decider reads the new
//! posture at its next `resolve`; the *advertisement* (`definitions()`) is
//! read at turn start, so the model's tool list catches up at the next turn.
//!
//! `bypass` is gated: it is reachable only where this process's
//! configuration allows it ([`SessionState::allow_bypass`], filled at launch
//! from the one source, `config::runtime::allow_bypass`). The gate refuses
//! reaching the mode; it never touches the modes already in force, and it
//! governs nothing structural — the SQL safety layer, the sandbox and the
//! allowlists answer every mode the same way.
//!
//! [`SessionPolicy`]: saya_agent::SessionPolicy
//! [`SessionState::allow_bypass`]: crate::interactive::session::state::SessionState::allow_bypass

use saya_agent::{AgentMode, ApprovalPolicy, PermissionMode};

use crate::interactive::session::commands::{approval_name, mode_message};
use crate::interactive::session::permissions::activation;
use crate::interactive::session::runtime::SessionRuntime;
use crate::interactive::session::state::SessionState;
use saya_store::BypassSource;

/// The mode's spelling — the one copy the switch answer, the refusal and the
/// status bar read, so no surface invents a name of its own.
pub(crate) const fn name(mode: PermissionMode) -> &'static str {
    match mode {
        PermissionMode::Ask => "ask",
        PermissionMode::AcceptEdits => "accept-edits",
        PermissionMode::Plan => "plan",
        PermissionMode::Bypass => "bypass",
    }
}

/// The keyboard cycle's order: `ask → accept-edits → plan → bypass → ask`,
/// with `bypass` skipped where the configuration withholds it. Shift+Tab
/// walks it through [`cycle_from`].
const CYCLE: [PermissionMode; 4] = [
    PermissionMode::Ask,
    PermissionMode::AcceptEdits,
    PermissionMode::Plan,
    PermissionMode::Bypass,
];

/// The grammar `/mode` parses under: the four mode names, plus `build` as an
/// alias for `ask` — the spelling the task-posture command used before the
/// two axes became one mode, kept so a habit still works. Case-sensitive
/// like `/approvals`; an unknown word is `None`, never a silent default.
pub(crate) fn parse(value: &str) -> Option<PermissionMode> {
    match value {
        "ask" | "build" => Some(PermissionMode::Ask),
        "accept-edits" => Some(PermissionMode::AcceptEdits),
        "plan" => Some(PermissionMode::Plan),
        "bypass" => Some(PermissionMode::Bypass),
        _ => None,
    }
}

/// The mode the keyboard cycle moves to from `current`: the next member of
/// [`CYCLE`], skipping `bypass` when `allow_bypass` is false. Skipping is
/// the whole point — a key that could reach `bypass` without the
/// configuration's consent would be a switch the `/mode` command refuses.
pub(crate) fn next_in_cycle(current: PermissionMode, allow_bypass: bool) -> PermissionMode {
    let start = CYCLE
        .iter()
        .position(|mode| *mode == current)
        .unwrap_or_default();
    for step in 1..=CYCLE.len() {
        let next = CYCLE[(start + step) % CYCLE.len()];
        if next != PermissionMode::Bypass || allow_bypass {
            return next;
        }
    }
    // Unreachable: `ask` is always a member and is never skipped. Answering
    // with the closed direction rather than panicking keeps a key press from
    // costing the session.
    PermissionMode::Ask
}

/// The session's permission mode: the pair the state carries, projected. A
/// consent posture with no mode of its own (`read-only`, `never`, set by
/// `/approvals`) projects to nothing — the surfaces that name a mode read
/// this, never the two raw strings, so they cannot disagree about which pair
/// is which mode.
pub(crate) fn current(state: &SessionState) -> Option<PermissionMode> {
    PermissionMode::of(
        state.approval_mode_parsed().ok()?,
        state.agent_mode_parsed(),
    )
}

/// The mode the keyboard cycle moves to from the session's own pair — the
/// next member of [`CYCLE`], `bypass` skipped where the configuration
/// withholds it.
///
/// A pair with no mode of its own has no place in the cycle: the press
/// enters at the cycle's closed default, `ask`, which asks for every call,
/// rather than jumping to a member the user never walked through.
pub(crate) fn cycle_from(state: &SessionState) -> PermissionMode {
    match current(state) {
        Some(mode) => next_in_cycle(mode, state.allow_bypass),
        None => PermissionMode::Ask,
    }
}

/// The refusal a withheld `bypass` answers with — the switch's and the key
/// cycle's one wording, naming the key that would allow it and the layer
/// that can never set it.
pub(crate) const BYPASS_REFUSAL: &str = "bypass is off: `[permissions] allow_bypass = true` in your \
     user configuration is the only way to allow it (a project's `.saya/config.toml` cannot).";

/// Switches the session's permission mode. Sets the pair on the state and on
/// the session's one policy, journals a fresh bypass activation (the moment
/// of consent, the same seam the launch and `/approvals` use), and answers
/// with the words the user reads — the mode, what it means, and, under
/// `bypass`, the activation line.
///
/// Refused for `bypass` while the session's configuration withholds it, and
/// refused by a frozen policy (a run's — stated before it starts); a refusal
/// changes nothing, so a refused switch never leaves half a posture behind.
pub(crate) fn switch(
    state: &mut SessionState,
    session: &mut SessionRuntime,
    next: PermissionMode,
) -> Result<String, String> {
    if next == PermissionMode::Bypass && !state.allow_bypass {
        return Err(BYPASS_REFUSAL.to_owned());
    }
    let (consent, agent_mode) = next.decompose();
    let before = state.approval_mode.clone();
    if !session.switch_policy(consent, agent_mode) {
        return Err("this session's approval policy is fixed for the run".to_owned());
    }
    // Leaving `plan` closes the gate: the approval the operator gave belonged
    // to a plan written for that posture, so a session that walks out of it
    // carries no standing permission back in — re-entering `plan` asks again.
    if agent_mode != AgentMode::Plan {
        session.universe().plan_gate().close();
    }
    if let Some(consent) = consent {
        state.approval_mode = approval_name(consent);
    }
    state.agent_mode = agent_mode.as_str().into();
    let mut message = mode_message(next);
    if activation::is_bypass_mode(state) {
        if activation::bypass_activated_by_command(&before, &state.approval_mode) {
            session.journal_bypass_activation(BypassSource::Command);
        }
        if let Some(line) = activation::line_if_bypass(state, &session.universe()) {
            message.push('\n');
            message.push_str(&line);
        }
    }
    Ok(message)
}

/// The mode a launch's `--permission-mode` states, or the typed refusal: an
/// unknown word, or `bypass` while this process's configuration withholds it.
/// The refusal is [`BYPASS_REFUSAL`] — the switch's own wording — so the flag
/// and `/mode` cannot refuse the same request in two different sentences.
pub(crate) fn stated_mode(spelling: &str, allow_bypass: bool) -> Result<PermissionMode, String> {
    let mode = parse(spelling).ok_or_else(|| {
        format!(
            "invalid --permission-mode `{spelling}`: expected ask, accept-edits, plan, or bypass"
        )
    })?;
    if mode == PermissionMode::Bypass && !allow_bypass {
        return Err(BYPASS_REFUSAL.to_owned());
    }
    Ok(mode)
}

/// The posture a launch runs under, from what the launch states and what the
/// record carried. `approval_explicit` is whether this process stated the
/// consent itself (`--approval-mode`) — an explicit statement is this
/// process's own consent and is never overridden. A *record* that says
/// `bypass` while this process's configuration withholds it degrades to
/// `ask`: the record cannot grant a consent the configuration refuses. The
/// degradation only ever narrows, so nothing is journalled for it.
pub(crate) fn launch_posture(
    approval: &str,
    agent: &str,
    approval_explicit: bool,
    allow_bypass: bool,
) -> (String, String) {
    let withheld =
        !allow_bypass && !approval_explicit && approval.parse() == Ok(ApprovalPolicy::Bypass);
    // Everything else rides verbatim, including a spelling no binary parses:
    // resume continuity is the record's, and the parse sites already fall
    // back to `ask` on their own.
    let approval = if withheld {
        approval_name(ApprovalPolicy::Ask)
    } else {
        approval.to_owned()
    };
    (approval, agent.to_owned())
}

#[cfg(test)]
#[path = "mode_tests.rs"]
mod tests;
