use crate::grant_token::{grant_token, session_answers_line};
use saya_agent::{ApprovalChoice, ApprovalDecision, ApprovalPolicy, SessionPolicy, ToolDefinition};

pub(crate) struct TerminalApproval {
    policy: SessionPolicy,
    can_prompt: bool,
}

impl TerminalApproval {
    pub(crate) fn new(policy: ApprovalPolicy, can_prompt: bool) -> Self {
        Self {
            policy: SessionPolicy::new(policy),
            can_prompt,
        }
    }

    /// Built over the session's one approval policy — the hoisted instance a
    /// session's turns clone, so a grant recorded through this decider's ask
    /// is in force for every later turn of the same session.
    pub(crate) fn from_session(policy: SessionPolicy, can_prompt: bool) -> Self {
        Self { policy, can_prompt }
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
        let grant = grant_token(&tool.name, arguments);
        match self.policy.resolve(&tool.effect, grant.as_deref()) {
            ApprovalDecision::Allow => true,
            ApprovalDecision::Deny => false,
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
