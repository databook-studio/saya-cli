use saya_agent::ApprovalPolicy;

pub(crate) struct TerminalApproval {
    policy: ApprovalPolicy,
    can_prompt: bool,
}

impl TerminalApproval {
    pub(crate) fn new(policy: ApprovalPolicy, can_prompt: bool) -> Self {
        Self { policy, can_prompt }
    }
}

#[async_trait::async_trait]
impl saya_agent::ApprovalDecider for TerminalApproval {
    async fn approve(&self, _: &saya_agent::ToolDefinition) -> bool {
        match self.policy {
            ApprovalPolicy::ReadOnly => true,
            ApprovalPolicy::Never => false,
            ApprovalPolicy::Ask if !self.can_prompt => false,
            ApprovalPolicy::Ask => {
                use std::io::{self, IsTerminal, Write};
                if !io::stdin().is_terminal() {
                    return false;
                }
                eprint!("Allow bounded read-only SQL query? [y/N] ");
                let _ = io::stderr().flush();
                let mut answer = String::new();
                io::stdin().read_line(&mut answer).is_ok()
                    && matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
            }
        }
    }
}
