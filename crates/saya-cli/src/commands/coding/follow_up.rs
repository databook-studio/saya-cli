//! Explicit bounded follow-up on a joined child's original owned checkout.
mod preparation;
pub(in crate::commands::coding) mod retained_root;
mod run;
use super::{CodingError, NativeCodingJob};
use saya_types::{AgentId, InvocationId, ManagedCheckoutId};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, num::NonZeroU64, sync::Arc};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChildFollowUpRequest {
    pub agent_id: AgentId,
    pub expected_completion_sequence: NonZeroU64,
    pub instruction: String,
}
#[derive(Debug, Serialize)]
pub struct ChildFollowUpStarted {
    pub agent_id: AgentId,
    pub previous_invocation_id: InvocationId,
    pub invocation_id: InvocationId,
    pub previous_completion_sequence: NonZeroU64,
    pub checkout_id: ManagedCheckoutId,
}

impl NativeCodingJob {
    pub async fn follow_up(
        self: &Arc<Self>,
        request: ChildFollowUpRequest,
    ) -> Result<ChildFollowUpStarted, CodingError> {
        let _operation = self.operation.try_lock().map_err(|_| CodingError::Busy)?;
        if self.active.lock().await.is_some() {
            return Err(CodingError::Busy);
        }
        let prepared = self.prepare_follow_up(request).await?;
        let agent = prepared.receipt.agent_id.clone();
        let controls = BTreeMap::from([(agent.clone(), prepared.node.control())]);
        let (live, _waiters) = self
            .open_session(vec![(agent, prepared.node)], controls)
            .await?;
        *self.active.lock().await = Some(live);
        Ok(prepared.receipt)
    }
}

#[cfg(all(test, unix))]
#[path = "follow_up/adoption_tests.rs"]
mod adoption_tests;
