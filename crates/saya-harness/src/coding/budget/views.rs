use super::{admission, ledger::State};
use async_trait::async_trait;
use saya_agent::{
    EffectAdmission, EffectAdmissionError, EffectOwnerFailure, EffectPermit, EffectSettlement,
    PhysicalAttempt, ToolEffect,
};
use saya_types::{AgentId, InvocationId};
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Clone)]
/// Root-scoped count view sharing the job ledger with all child views.
/// Physical attempts bind to the job-root actor and its exact model permit.
pub struct RootBudgetView {
    pub(super) inner: Arc<Mutex<State>>,
}

#[derive(Clone)]
pub struct ChildBudgetView {
    pub(super) inner: Arc<Mutex<State>>,
    pub(super) agent: AgentId,
    pub(super) invocation: InvocationId,
}

#[async_trait]
impl EffectAdmission for RootBudgetView {
    async fn admit_model_request(&self) -> Result<EffectPermit, EffectAdmissionError> {
        admission::admit_model(&mut self.inner.lock().await, None, None, None).await
    }

    async fn admit_tool_batch(
        &self,
        effects: &[ToolEffect],
    ) -> Result<Vec<EffectPermit>, EffectAdmissionError> {
        super::tools::admit_tools(&mut self.inner.lock().await, effects, None, None, None).await
    }

    async fn admit_physical_attempt(
        &self,
        attempt: PhysicalAttempt,
    ) -> Result<EffectPermit, EffectAdmissionError> {
        admission::admit_physical(&mut self.inner.lock().await, attempt, None, None).await
    }

    async fn settle(
        &self,
        permit: EffectPermit,
        settlement: EffectSettlement,
    ) -> Result<(), EffectOwnerFailure> {
        super::settlement::finish_effect(&mut self.inner.lock().await, None, permit, settlement)
            .await
    }
}

#[async_trait]
impl EffectAdmission for ChildBudgetView {
    async fn admit_model_request(&self) -> Result<EffectPermit, EffectAdmissionError> {
        let mut state = self.inner.lock().await;
        if !state
            .local
            .contains_key(&(self.agent.clone(), self.invocation.clone()))
        {
            return Err(state.owner_failure());
        }
        let limit = state
            .local
            .get(&(self.agent.clone(), self.invocation.clone()))
            .and_then(|local| local.limits)
            .map(|limits| limits.logical_requests);
        admission::admit_model(
            &mut state,
            Some(self.agent.clone()),
            Some(self.invocation.clone()),
            limit,
        )
        .await
    }

    async fn admit_tool_batch(
        &self,
        effects: &[ToolEffect],
    ) -> Result<Vec<EffectPermit>, EffectAdmissionError> {
        let mut state = self.inner.lock().await;
        if !state
            .local
            .contains_key(&(self.agent.clone(), self.invocation.clone()))
        {
            return Err(state.owner_failure());
        }
        let limit = state
            .local
            .get(&(self.agent.clone(), self.invocation.clone()))
            .and_then(|local| local.limits)
            .map(|limits| limits.tool_calls);
        super::tools::admit_tools(
            &mut state,
            effects,
            Some(self.agent.clone()),
            Some(self.invocation.clone()),
            limit,
        )
        .await
    }

    async fn admit_physical_attempt(
        &self,
        attempt: PhysicalAttempt,
    ) -> Result<EffectPermit, EffectAdmissionError> {
        let mut state = self.inner.lock().await;
        if !state
            .local
            .contains_key(&(self.agent.clone(), self.invocation.clone()))
        {
            return Err(state.owner_failure());
        }
        admission::admit_physical(
            &mut state,
            attempt,
            Some(self.agent.clone()),
            Some(self.invocation.clone()),
        )
        .await
    }

    async fn settle(
        &self,
        permit: EffectPermit,
        settlement: EffectSettlement,
    ) -> Result<(), EffectOwnerFailure> {
        super::settlement::finish_effect(
            &mut self.inner.lock().await,
            Some((&self.agent, &self.invocation)),
            permit,
            settlement,
        )
        .await
    }
}
