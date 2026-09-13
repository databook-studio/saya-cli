use crate::grant_token::{TurnPrimary, grant_token, session_answers_line};
use saya_agent::{ApprovalChoice, ApprovalDecision, ApprovalPolicy, SessionPolicy, ToolDefinition};

pub(crate) struct TerminalApproval {
    policy: SessionPolicy,
    can_prompt: bool,
    /// The turn's primary connection, bound by the turn that owns this
    /// decider; the SQL family's suggestion names it when the call names
    /// no connection. Unbound — a run's construction — suggests no token.
    primary: TurnPrimary,
}

impl TerminalApproval {
    pub(crate) fn new(policy: ApprovalPolicy, can_prompt: bool, primary: TurnPrimary) -> Self {
        Self {
            policy: SessionPolicy::new(policy),
            can_prompt,
            primary,
        }
    }

    /// The headless run's decider (U4: the same engine, frozen): built over a
    /// [`SessionPolicy::frozen`] seeded from the run's `--allow` tokens — the
    /// stated scopes are the approval — so a seed pre-answers the asks it
    /// names and everything else an `ask` mode would raise denies with the
    /// engine's own reason. It prompts nothing and records nothing: a
    /// headless session grant is impossible, not merely unused. The primary
    /// stays unbound — the run's fail-closed rule: only a call that names its
    /// connection suggests a token, never a guessed one.
    pub(crate) fn frozen(mode: ApprovalPolicy, seeds: &[String]) -> Self {
        Self {
            policy: SessionPolicy::frozen(mode, seeds),
            can_prompt: false,
            primary: TurnPrimary::default(),
        }
    }

    /// Built over the session's one approval policy — the hoisted instance a
    /// session's turns clone, so a grant recorded through this decider's ask
    /// is in force for every later turn of the same session. The primary is
    /// the turn's handle: the turn binds the registry's primary into it
    /// before the model runs.
    pub(crate) fn from_session(
        policy: SessionPolicy,
        can_prompt: bool,
        primary: TurnPrimary,
    ) -> Self {
        Self {
            policy,
            can_prompt,
            primary,
        }
    }
}

/// The prompt shown when an `Ask` approval needs the user: for tools whose call
/// has a visible detail, that detail and the SQL sentence; for every other
/// tool, a generic sentence naming the tool, so nothing is ever approved under
/// a sentence it does not match. The answers line follows: the session grant
/// answer names the offered token only when one exists.
pub(crate) fn approval_prompt(
    tool: &ToolDefinition,
    arguments: &serde_json::Value,
    grant: Option<&str>,
) -> String {
    let answers = session_answers_line(grant);
    match crate::agent::tools::tool_call_detail(&tool.name, arguments) {
        Some(detail) => format!("  {detail}\nAllow bounded read-only SQL query? {answers} "),
        None => format!("Run tool `{}`? {answers} ", tool.name),
    }
}

/// The user's typed answer mapped onto a [`ApprovalChoice`]. The habit and the
/// script keep their meaning — `y`/`yes` allow once, `n`/`no` deny — `a` is
/// the allow-once answer's key, `s` grants exactly the offered token (and is
/// an unoffered deny when no token exists), and anything unrecognised denies.
pub(crate) fn terminal_choice(answer: &str, grant: Option<&str>) -> ApprovalChoice {
    match answer.trim().to_ascii_lowercase().as_str() {
        "y" | "yes" | "a" => ApprovalChoice::AllowOnce,
        "s" => match grant {
            Some(token) => ApprovalChoice::AllowSession {
                token: token.to_owned(),
            },
            None => ApprovalChoice::Deny,
        },
        _ => ApprovalChoice::Deny,
    }
}

#[async_trait::async_trait]
impl saya_agent::ApprovalDecider for TerminalApproval {
    async fn approve(&self, tool: &ToolDefinition, arguments: &serde_json::Value) -> bool {
        let primary = self.primary.get();
        let grant = grant_token(&tool.name, arguments, primary.as_deref());
        match self.policy.resolve(&tool.effect, grant.as_deref()) {
            ApprovalDecision::Allow => true,
            ApprovalDecision::Deny { .. } => false,
            ApprovalDecision::Ask if !self.can_prompt => false,
            ApprovalDecision::Ask => {
                use std::io::{self, IsTerminal, Write};
                if !io::stdin().is_terminal() {
                    return false;
                }
                eprint!("{}", approval_prompt(tool, arguments, grant.as_deref()));
                let _ = io::stderr().flush();
                let mut answer = String::new();
                if !(io::stdin().read_line(&mut answer).is_ok()) {
                    return false;
                }
                let choice = terminal_choice(&answer, grant.as_deref());
                // Only "allow for this session" records anything; the grant
                // lands in the session's one policy, so it outlives the turn.
                self.policy.record(choice.clone());
                !matches!(choice, ApprovalChoice::Deny)
            }
        }
    }
}
