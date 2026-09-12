use saya_agent::{ApprovalDecision, ApprovalPolicy, SessionPolicy, ToolDefinition};

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
}

/// The prompt shown when an `Ask` approval needs the user: for tools whose call
/// has a visible detail, that detail and the SQL sentence; for every other
/// tool, a generic sentence naming the tool, so nothing is ever approved under
/// a sentence it does not match.
pub(crate) fn approval_prompt(tool: &ToolDefinition, arguments: &serde_json::Value) -> String {
    match crate::agent::tools::tool_call_detail(&tool.name, arguments) {
        Some(detail) => format!("  {detail}\nAllow bounded read-only SQL query? [y/N] "),
        None => format!("Run tool `{}`? [y/N] ", tool.name),
    }
}

#[async_trait::async_trait]
impl saya_agent::ApprovalDecider for TerminalApproval {
    async fn approve(&self, tool: &ToolDefinition, arguments: &serde_json::Value) -> bool {
        match self.policy.resolve(&tool.effect, None) {
            ApprovalDecision::Allow => true,
            ApprovalDecision::Deny => false,
            ApprovalDecision::Ask if !self.can_prompt => false,
            ApprovalDecision::Ask => {
                use std::io::{self, IsTerminal, Write};
                if !io::stdin().is_terminal() {
                    return false;
                }
                eprint!("{}", approval_prompt(tool, arguments));
                let _ = io::stderr().flush();
                let mut answer = String::new();
                io::stdin().read_line(&mut answer).is_ok()
                    && matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
            }
        }
    }
}
