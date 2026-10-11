//! Fresh host consent for a root ceiling. The native application separately
//! renews checkout ownership before/after consent and at each effect boundary.
use super::{AuthorityError, RootDelegationInput, RootDelegationScope};
use saya_agent::{ApprovalDecider, LocalStateEffect, ToolConcurrency, ToolDefinition, ToolEffect};
use serde_json::{Value, json};

impl RootDelegationInput {
    /// Exact data shown to the host decider. These bytes are not an approval.
    pub fn approval_request(&self) -> Value {
        json!({
            "job_id":self.job_id,"agent_id":self.agent_id,"invocation_id":self.invocation_id,
            "repository_id":self.repository_id,"snapshot_id":self.snapshot_id,
            "checkout_id":self.managed_checkout_id,"policy_digest":self.policy_digest,
            "revocation_epoch":self.revocation_epoch,"write_paths":self.write_paths,
            "capabilities":self.capabilities,"database_profiles":self.database_profiles,
        })
    }
}
impl RootDelegationScope {
    /// Issues a ceiling only after a fresh decision on every identity and scope.
    /// Host deciders are trusted compiled application code; child views, wire
    /// receipts and audit rows alone cannot reconstruct an approved root.
    /// This is consent provenance, not a cached proof of checkout ownership.
    pub async fn approve(
        input: RootDelegationInput,
        decider: &dyn ApprovalDecider,
    ) -> Result<Self, AuthorityError> {
        let request = input.approval_request();
        let scope = Self::from_input(input)?;
        let definition = ToolDefinition {
            name: "coding_root_scope".into(),
            description: "Issue this exact native root scope after fresh host consent".into(),
            read_only: false,
            parameters: json!({"type":"object"}),
            effect: ToolEffect {
                database_data: false,
                external_side_effect: false,
                requires_approval: true,
                local_state: LocalStateEffect::WriteWorkspace,
            },
            concurrency: ToolConcurrency::Serial,
            completion: None,
        };
        if !decider.approve(&definition, &request).await {
            return Err(AuthorityError::ApprovalDenied);
        }
        Ok(scope)
    }
}
