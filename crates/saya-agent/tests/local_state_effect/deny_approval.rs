/// A decider that refuses every call, so a `requires_approval` tool is always
/// denied at the prompt — used to prove the approval gate still fires for an
/// external-side-effect tool that set `requires_approval`.
struct DenyApproval;
#[async_trait]
impl ApprovalDecider for DenyApproval {
    async fn approve(&self, _: &ToolDefinition, _: &serde_json::Value) -> bool {
        false
    }
}
