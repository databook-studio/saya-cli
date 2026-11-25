//! The lane decision: where a call runs, why it was refused, and the one
//! resolution `approve` and `execute` both read.

use std::path::PathBuf;

use saya_agent::{ApprovalPolicy, SessionGrants};
use saya_config::{CommandDecision, CommandRules};
use serde_json::Value;

use super::ModeCell;
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
    /// The host, with whether something other than the operator pre-answered
    /// the call: a permission rule, or a `shell:` grant this job remembered
    /// from an earlier "yes for this run". Either way the call reaches the
    /// host under `ask` without reaching the operator; every other host call
    /// reaches the mode's own decision.
    Host(bool),
    /// Refuse, for this reason.
    Refused(String),
}

/// The lane decision and the operator that answers the host lane: the part of
/// a shell that does not depend on which workspace it runs in, shared by every
/// surface that advertises `shell`.
pub(crate) struct ShellPolicy {
    /// The mode every call resolves — the job's shared cell where the composing
    /// surface handed one, and this policy's own otherwise.
    mode: ModeCell,
    /// The operator's approver for the host lane, when one is present. Its
    /// operator also holds the job's remembered grants, which pre-answer a call
    /// the mode would otherwise ask about.
    prompt: Option<Prompt>,
    /// Whether the shell this policy decides for holds a sandbox lane.
    has_sandbox: bool,
    /// Whether the composed shell holds a host lane at all: the lane a
    /// remembered grant names, and the only one it may pre-answer. A shell
    /// composed under `read-only` holds none.
    host_lane: bool,
    /// The trusted command-prefix rules. A deny rule refuses in every mode;
    /// an allow rule pre-answers the host lane under `ask`.
    rules: CommandRules,
    /// The surface's own "yes for this run" store, where it keeps one
    /// independently of an operator — a session binds it per turn. Read by
    /// [`remembered`](Self::remembered) beside the prompt's store.
    grants: Option<SessionGrants>,
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
            mode: ModeCell::new(policy),
            prompt,
            has_sandbox,
            // The composition adds the host lane in every mode that serves it:
            // always over the host alone, and over a proven sandbox in every
            // mode but `read-only`, which has nothing outside it to reach.
            host_lane: !has_sandbox || policy != ApprovalPolicy::ReadOnly,
            rules,
            grants: None,
            workspace,
        }
    }

    /// Binds the surface's own store: the grants it consults beside whatever
    /// the operator remembers. The session shell the deleted body composed was
    /// its production caller; the grant tests bind a store here directly.
    #[cfg(test)]
    pub(crate) fn with_grants(mut self, grants: Option<SessionGrants>) -> Self {
        self.grants = grants;
        self
    }

    /// Follows a caller's shared mode cell: every call resolves the mode the
    /// cell holds, not the one this policy was composed under.
    pub(crate) fn following(mut self, mode: ModeCell) -> Self {
        self.mode = mode;
        self
    }

    /// Whether this call asks for the host lane: `sandbox` is omitted or
    /// false, or this host has no sandbox lane to give it.
    pub(crate) fn wants_host(&self, arguments: &Value) -> bool {
        arguments.get("sandbox") != Some(&Value::Bool(true)) || !self.has_sandbox
    }

    /// The command-prefix decision for this call, if the call names one.
    ///
    /// The vetted read-only set skips the operator's question, so it has to
    /// read what the workspace tools would read: a command naming a path the
    /// read tool refuses — [`saya_config::sensitive_path`], the store's screen,
    /// or the shared redactor, which is the one [`saya_harness::workspace::
    /// private_path`] rule — keeps its question. The configuration crate states
    /// the names the set reads ([`saya_config::named_paths`]); the screen those
    /// names are read against lives here, because a configuration crate cannot
    /// see the harness's list.
    fn decision<'a>(&'a self, arguments: &Value) -> CommandDecision<'a> {
        let Some(command) = arguments.get("command").and_then(Value::as_str) else {
            return CommandDecision::Unmatched;
        };
        let decision = saya_config::decide(&self.rules, command, self.workspace.as_deref());
        if decision == CommandDecision::ReadOnly
            && saya_config::named_paths(command)
                .iter()
                .any(|path| saya_harness::workspace::private_path(path))
        {
            return CommandDecision::Unmatched;
        }
        decision
    }

    /// Whether this command changes the checkout's git directory, as an
    /// argument or a redirect target, without being one of the vetted
    /// read-only commands. A read-only command cannot change the git
    /// directory, so the set keeps its question-skipping answer; every other
    /// command that names one is put to the operator however the rules and the
    /// run's remembered answers read. The same reading the write tools use
    /// decides the names, so a `.git` pointer file's gitdir is covered here
    /// too.
    fn git_write(&self, arguments: &Value) -> bool {
        if self.decision(arguments) == CommandDecision::ReadOnly {
            return false;
        }
        let Some(command) = arguments.get("command").and_then(Value::as_str) else {
            return false;
        };
        saya_config::named_paths(command)
            .iter()
            .any(|path| super::super::git_write::targets_git_dir(self.workspace.as_deref(), path))
    }

    /// Whether this job's remembered answers pre-answer this call without the
    /// operator: the call's own `shell:` token is one the store holds, or — for
    /// one release — the old `run_command` spelling's `command:<program>` token
    /// is, for a command that is that one program (see
    /// [`legacy_program_token`](crate::grant_token::legacy_program_token)).
    /// Read from the asking arm of [`lane`](Self::lane) only, so a deny rule and
    /// the mode have each decided first, and only for a shell that holds the
    /// host lane the token names. The store is the surface's own where it keeps
    /// one (a session binds it per turn, and a headless turn composes no prompt
    /// at all) and the operator's otherwise (a job's). A command that is not one
    /// plain segment names no token, so it never matches.
    fn remembered(&self, arguments: &Value) -> bool {
        if !self.host_lane {
            return false;
        }
        let own = crate::grant_token::shell_token_for_host_route(arguments)
            .is_some_and(|token| self.granted(&token));
        own || crate::grant_token::legacy_program_token(arguments)
            .is_some_and(|token| self.granted(&token))
    }

    /// Whether either store this shell consults remembers `token`.
    fn granted(&self, token: &str) -> bool {
        self.grants
            .as_ref()
            .is_some_and(|grants| grants.is_granted(token))
            || self
                .prompt
                .as_ref()
                .is_some_and(|prompt| prompt.granted(token))
    }

    /// The lane this call takes, decided once. The mode is read from the
    /// policy's cell on every call, so a mode stated between two calls decides
    /// the second. A deny rule refuses in every mode, `bypass` included. An
    /// allow rule that covers every segment of a substitution-free command
    /// pre-answers the host lane under the asking modes; it never widens
    /// `read-only` or `never`. A command that changes the checkout's git
    /// directory is never pre-answered — neither by a rule nor by a remembered
    /// answer — so the operator sees it in the asking modes. A command the
    /// vetted read-only set answers
    /// ([`CommandDecision::ReadOnly`]) skips the same question — but only where
    /// an operator is there to answer it: the set skips a question, and with
    /// nobody to ask the lane falls back exactly as before. A `shell:` grant
    /// the job's operator remembered
    /// pre-answers the same way, and only here — after the rule and after the
    /// mode, for a call the mode would otherwise put to the operator. Every
    /// other call is decided as before: a call that named the sandbox on a host
    /// that holds one runs there, unasked; the two asking modes (`ask`,
    /// `accept-edits`) run a host call when an operator is there to answer and
    /// otherwise fall back to the sandbox when
    /// one is proven, carrying the reason no operator could be asked, and
    /// refusing when none is; `read-only` serves it from the sandbox with the
    /// root write-denied, refusing when no sandbox is proven; and `never` has
    /// no shell to run it in.
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
        match self.mode.get() {
            ApprovalPolicy::Bypass => Lane::Host(rule_allowed),
            ApprovalPolicy::Ask | ApprovalPolicy::AcceptEdits => {
                // A write into the git directory is never pre-answered by a
                // rule or a remembered answer: the operator sees it.
                let pre_answered =
                    !self.git_write(arguments) && (rule_allowed || self.remembered(arguments));
                // The vetted read-only set skips a question, not the consent
                // path: with no operator to skip, today's fallback stands.
                let read_only =
                    self.prompt.is_some() && self.decision(arguments) == CommandDecision::ReadOnly;
                if pre_answered || self.prompt.is_some() {
                    Lane::Host(pre_answered || read_only)
                } else if self.has_sandbox {
                    Lane::Sandbox(Some(SANDBOX_DOWNGRADE_NOTICE))
                } else {
                    Lane::Refused(HOST_APPROVAL_UNAVAILABLE.to_owned())
                }
            }
            ApprovalPolicy::ReadOnly if self.has_sandbox => Lane::Sandbox(None),
            ApprovalPolicy::ReadOnly => Lane::Refused(READ_ONLY_REFUSAL.to_owned()),
            ApprovalPolicy::Never => Lane::Refused(NEVER_REFUSAL.to_owned()),
        }
    }

    /// Decides one call. The lane [`lane`](Self::lane) names is allowed
    /// outright — a sandboxed call because the confinement is the consent, a
    /// pre-answered host call because the operator stated the prefix or the job
    /// remembered the answer, and a host call under `bypass` because the mode
    /// said so — and only the host lane under an asking mode reaches the
    /// operator, which is told who is asking.
    pub(crate) async fn approve(&self, arguments: &Value) -> bool {
        match self.lane(arguments) {
            Lane::Sandbox(_) => true,
            Lane::Host(pre_answered) => {
                if pre_answered {
                    return true;
                }
                // `lane` names the host under an asking mode only when an
                // operator is there to answer, so those are the modes that
                // ask. Any other mode naming the host (`bypass`) allowed it
                // outright.
                if !matches!(
                    self.mode.get(),
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
    /// present. The sandbox lane and a pre-answered host call are automatic.
    pub(crate) fn would_ask(&self, arguments: &Value) -> bool {
        match self.lane(arguments) {
            Lane::Host(pre_answered) => {
                !pre_answered
                    && matches!(
                        self.mode.get(),
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
