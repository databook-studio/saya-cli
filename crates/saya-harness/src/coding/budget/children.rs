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
        let mut state = self.inner.lock().await;
        if state
            .local
            .contains_key(&(agent.clone(), invocation.clone()))
        {
            return Err(state.owner_failure());
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
                limits: Some(limits),
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
        if outcome != EffectOutcome::Unknown {
            state.counts.child_slots = state
                .counts
                .child_slots
                .checked_sub(1)
                .ok_or_else(|| state.owner_failure())?;
        }
        state.local.remove(&key);
        super::tools::finish_open_groups(&mut state, &key).await?;
        Ok(())
    }

    pub async fn child_started(&self, agent: &AgentId, invocation: &InvocationId) -> bool {
        self.inner
            .lock()
            .await
            .local
            .get(&(agent.clone(), invocation.clone()))
            .is_some_and(|counts| counts.logical > 0)
    }
}
