//! The lane decision: where a call runs, why it was refused, and the one
//! resolution `approve` and `execute` both read.

use std::path::PathBuf;

use saya_agent::ApprovalPolicy;
use saya_config::{CommandDecision, CommandRules};
use serde_json::Value;

use crate::agent::tools::consent::Prompt;

/// The refusal a host call earns under `ask` when no operator is present to
/// answer it and no sandbox is proven to fall back to. The way to the host from
/// a driver that cannot be asked is named, so the refusal is actionable rather
/// than merely true — and named for the surfaces that have it: `saya run`
/// refuses `bypass` outright, so a run is not promised a lane it cannot take.
pub(crate) const HOST_APPROVAL_UNAVAILABLE: &str = "running on the host needs the user's approval \
     and no operator is available here: reach it with --approval-mode bypass, or allow this \
     command with --allow-command <prefix>";

/// What a host call under `ask` is told when no operator could answer it and
/// the proven sandbox took the call instead. The reroute is stated in the
/// result, so a command that fails for want of network is attributable to the
/// lane it ran in rather than read as a host failure.
pub(crate) const SANDBOX_DOWNGRADE_NOTICE: &str = "no operator could approve a host call, so it \
     ran in the sandbox: no network, writes confined to the checkout. Reach the host with \
     --approval-mode bypass or --allow-command <prefix>";

/// The refusal a host call earns under `read-only` when no sandbox is proven,
/// where the shell would exist only inside one.
pub(crate) const READ_ONLY_REFUSAL: &str = "this session is read-only: the shell runs only inside the sandbox, where nothing in the \
     checkout is writable";

/// The refusal a call earns under `never`. A surface advertises no shell in
/// that mode at all — [`compose`](super::compose::compose) returns `None` — so a built shell never
/// reaches this; it is here so the lane decision stays total.
pub(crate) const NEVER_REFUSAL: &str = "this session runs no shell";

/// The lane one call takes: the sandbox, the host, or a refusal. Resolved
/// once by [`ShellPolicy::lane`] and read by both `approve` and `execute`, so
/// the two can never disagree about where a call runs or why it was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Lane {
    /// The proven sandbox: no prompt, writes confined to the checkout. The
    /// notice is present only where a host call was rerouted here because no
    /// operator could answer it — the reason the model is owed, so a failure
    /// for want of network is attributable to this lane. A call that asked for
    /// the sandbox, and a mode that serves it by its own rule (`read-only`),
    /// carry `None`: nothing was taken from them.
    Sandbox(Option<&'static str>),
    /// The host, with whether a permission rule pre-answered the call. A
    /// rule-allowed call takes this lane under `ask` without reaching the
    /// operator; every other host call reaches the mode's own decision.
    Host(bool),
    /// Refuse, for this reason.
    Refused(String),
}

/// The lane decision and the operator that answers the host lane: the part of
/// a shell that does not depend on which workspace it runs in, shared by every
/// surface that advertises `shell`.
pub(crate) struct ShellPolicy {
    policy: ApprovalPolicy,
    /// The operator's approver for the host lane, when one is present.
    prompt: Option<Prompt>,
    /// Whether the shell this policy decides for holds a sandbox lane.
    has_sandbox: bool,
    /// The trusted command-prefix rules. A deny rule refuses in every mode;
    /// an allow rule pre-answers the host lane under `ask`.
    rules: CommandRules,
    /// The workspace root, when one is known, bounding the leading `cd` the
    /// rule matcher may strip.
    workspace: Option<PathBuf>,
}

impl ShellPolicy {
    pub(crate) fn new(
        policy: ApprovalPolicy,
        prompt: Option<Prompt>,
        has_sandbox: bool,
        rules: CommandRules,
        workspace: Option<PathBuf>,
    ) -> Self {
        Self {
            policy,
            prompt,
            has_sandbox,
            rules,
            workspace,
        }
    }

    /// Whether this call asks for the host lane: `sandbox` is omitted or
    /// false, or this host has no sandbox lane to give it.
    pub(crate) fn wants_host(&self, arguments: &Value) -> bool {
        arguments.get("sandbox") != Some(&Value::Bool(true)) || !self.has_sandbox
    }

    /// The command-prefix decision for this call, if the call names one.
    fn decision<'a>(&'a self, arguments: &Value) -> CommandDecision<'a> {
        match arguments.get("command").and_then(Value::as_str) {
            Some(command) => saya_config::decide(&self.rules, command, self.workspace.as_deref()),
            None => CommandDecision::Unmatched,
        }
    }

    /// The lane this call takes, decided once. A deny rule refuses in every
    /// mode, `bypass` included. An allow rule that covers every segment of a
    /// substitution-free command pre-answers the host lane under the asking
    /// modes; it never widens `read-only` or `never`. Every other call is
    /// decided as before: a call that named the sandbox on a host that holds
    /// one runs there, unasked; the two asking modes (`ask`, `accept-edits`)
    /// run a host call when an operator is there to answer and otherwise fall
    /// back to the sandbox when one is proven, carrying the reason no operator
    /// could be asked, and refusing when none is; `read-only` serves it from
    /// the sandbox with the root write-denied, refusing when no sandbox is
    /// proven; and `never` has no shell to run it in.
    pub(crate) fn lane(&self, arguments: &Value) -> Lane {
        if let CommandDecision::Deny(rule) = self.decision(arguments) {
            return Lane::Refused(format!(
                "the command is denied by the permission rule `{}`",
                rule.text()
            ));
        }
        if !self.wants_host(arguments) {
            return Lane::Sandbox(None);
        }
        let rule_allowed = self.decision(arguments) == CommandDecision::Allow;
        match self.policy {
            ApprovalPolicy::Bypass => Lane::Host(rule_allowed),
            ApprovalPolicy::Ask | ApprovalPolicy::AcceptEdits
                if rule_allowed || self.prompt.is_some() =>
            {
                Lane::Host(rule_allowed)
            }
            ApprovalPolicy::Ask | ApprovalPolicy::AcceptEdits if self.has_sandbox => {
                Lane::Sandbox(Some(SANDBOX_DOWNGRADE_NOTICE))
            }
            ApprovalPolicy::Ask | ApprovalPolicy::AcceptEdits => {
                Lane::Refused(HOST_APPROVAL_UNAVAILABLE.to_owned())
            }
            ApprovalPolicy::ReadOnly if self.has_sandbox => Lane::Sandbox(None),
            ApprovalPolicy::ReadOnly => Lane::Refused(READ_ONLY_REFUSAL.to_owned()),
            ApprovalPolicy::Never => Lane::Refused(NEVER_REFUSAL.to_owned()),
        }
    }

    /// Decides one call. The lane [`lane`](Self::lane) names is allowed
    /// outright — a sandboxed call because the confinement is the consent, a
    /// rule-allowed host call because the operator stated the prefix, and a
    /// host call under `bypass` because the mode said so — and only the host
    /// lane under an asking mode reaches the operator, which is told who is
    /// asking.
    pub(crate) async fn approve(&self, arguments: &Value) -> bool {
        match self.lane(arguments) {
            Lane::Sandbox(_) => true,
            Lane::Host(rule_allowed) => {
                if rule_allowed {
                    return true;
                }
                // `lane` names the host under an asking mode only when an
                // operator is there to answer, so those are the modes that
                // ask. Any other mode naming the host (`bypass`) allowed it
                // outright.
                if !matches!(
                    self.policy,
                    ApprovalPolicy::Ask | ApprovalPolicy::AcceptEdits
                ) {
                    return true;
                }
                match &self.prompt {
                    Some(prompt) => prompt.ask(arguments).await,
                    // Unreachable: `lane` names the host under `ask` only
                    // with an operator. Deny rather than guess.
                    None => false,
                }
            }
            Lane::Refused(_) => false,
        }
    }

    /// Whether [`approve`](Self::approve) would put this call to the operator:
    /// only an unallowed host call under an asking mode with an operator
    /// present. The sandbox lane and a rule-allowed host call are automatic.
    pub(crate) fn would_ask(&self, arguments: &Value) -> bool {
        match self.lane(arguments) {
            Lane::Host(rule_allowed) => {
                !rule_allowed
                    && matches!(
                        self.policy,
                        ApprovalPolicy::Ask | ApprovalPolicy::AcceptEdits
                    )
                    && self.prompt.is_some()
            }
            Lane::Sandbox(_) | Lane::Refused(_) => false,
        }
    }

    /// The typed refusal this call earns, or `None` where it does not.
    pub(crate) fn refusal_detail(&self, arguments: &Value) -> Option<String> {
        match self.lane(arguments) {
            Lane::Refused(reason) => Some(reason),
            Lane::Sandbox(_) | Lane::Host(_) => None,
        }
    }
}
