/// A decider that grants every call — the counterpart of `DenyApproval`, used
/// where the point is the loop's gates *after* approval was granted, not the
/// approval policy itself.
struct AllowApproval;
#[async_trait]
impl ApprovalDecider for AllowApproval {
    async fn approve(&self, _: &ToolDefinition, _: &serde_json::Value) -> bool {
        true
    }
}
