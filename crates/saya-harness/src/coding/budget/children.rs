use super::ledger::{ChildCountLimits, ChildSession, JobBudgetLedger, LocalCounts};
use saya_agent::{EffectAdmissionError, EffectOutcome};
use saya_types::{AgentId, BudgetDimension, InvocationId};

impl JobBudgetLedger {
    pub async fn begin_child(
        &self,
        agent: &AgentId,
        invocation: &InvocationId,
        limits: ChildCountLimits,
    ) -> Result<ChildSession, EffectAdmissionError> {
        if limits.logical_requests == 0 || limits.tool_calls == 0 {
            return Err(EffectAdmissionError::Denied);
        }
        self.begin_child_with_limits(agent, invocation, Some(limits))
            .await
    }

    /// Starts a child whose aggregate request/tool limits are absent in the
    /// authenticated uncapped policy. Finite child slots still apply.
    pub async fn begin_child_uncapped(
        &self,
        agent: &AgentId,
        invocation: &InvocationId,
    ) -> Result<ChildSession, EffectAdmissionError> {
        self.begin_child_with_limits(agent, invocation, None).await
    }

    async fn begin_child_with_limits(
        &self,
        agent: &AgentId,
        invocation: &InvocationId,
        limits: Option<ChildCountLimits>,
    ) -> Result<ChildSession, EffectAdmissionError> {
        let mut state = self.inner.lock().await;
        if state.policy.uncapped_v2().is_some() != limits.is_none() {
            return Err(EffectAdmissionError::Denied);
        }
        // One active turn per logical child, including attempted invocation rotation.
        // Ordinary contention must not poison the owner or cancel its first turn.
        if state
            .local
            .keys()
            .any(|(active_agent, _)| active_agent == agent)
        {
            return Err(EffectAdmissionError::Denied);
        }
        let permit = state
            .admit_one(
                BudgetDimension::ChildSlots,
                1,
                Some(agent.clone()),
                Some(invocation.clone()),
            )
            .await?;
        state.local.insert(
            (agent.clone(), invocation.clone()),
            LocalCounts {
                limits,
                ..LocalCounts::default()
            },
        );
        Ok(ChildSession {
            permit,
            view: self.child(agent.clone(), invocation.clone()),
        })
    }

    pub async fn finish_child(
        &self,
        session: ChildSession,
        outcome: EffectOutcome,
    ) -> Result<(), EffectAdmissionError> {
        let mut state = self.inner.lock().await;
        if !std::sync::Arc::ptr_eq(&session.view.inner, &self.inner) {
            return Err(state.owner_failure());
        }
        let key = (session.view.agent.clone(), session.view.invocation.clone());
        if !state.local.contains_key(&key) {
            return Err(state.owner_failure());
        }
        state
            .finish(
                session.permit,
                super::settlement::Settlement::Child(outcome),
            )
            .await?;
        if outcome == EffectOutcome::Unknown {
            state.unknown_children.insert(key.clone());
        } else {
            state.counts.child_slots = state
                .counts
                .child_slots
                .checked_sub(1)
                .ok_or_else(|| state.owner_failure())?;
            state.local.remove(&key);
        }
        super::tools::finish_open_groups(&mut state, &key).await?;
        Ok(())
    }

    /// Whether the child still has admitted work or settled an unknown tool
    /// effect, so a join cannot treat its effects as settled. Unknown usage of
    /// a model request or its transport attempts does not count: a model
    /// request cannot change the workspace.
    pub async fn child_started(&self, agent: &AgentId, invocation: &InvocationId) -> bool {
        let state = self.inner.lock().await;
        state
            .local
            .get(&(agent.clone(), invocation.clone()))
            .is_some_and(|counts| counts.logical > 0)
            || state
                .unknown_children
                .contains(&(agent.clone(), invocation.clone()))
    }
}
