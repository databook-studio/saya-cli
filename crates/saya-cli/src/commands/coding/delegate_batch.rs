//! A delegation registers every child it names, or none.
use super::{
    CodingError, CodingTask, NativeCodingJob, context, delegate::StagedChild,
    task_history::MAX_TASK_HISTORY_BYTES,
};
use saya_types::{AgentId, ChildInvocationRecord, InvocationId};

impl NativeCodingJob {
    /// Registers, checks out and composes the child of every task, binds each
    /// child's scope, then records the batch in the task history. A failure at any
    /// step takes back every registration this call made and writes no history,
    /// so no child is left queued with no schedule to run it. Checkouts and ledger
    /// events are append-only and stay; a retry that names the same agent adopts
    /// its checkout.
    pub(super) async fn stage_delegation(
        &self,
        tasks: Vec<CodingTask>,
    ) -> Result<Vec<StagedChild>, CodingError> {
        let mut registered = Vec::new();
        match self.stage_children(tasks, &mut registered).await {
            Ok(staged) => Ok(staged),
            Err(error) => {
                self.unregister(&registered).await;
                Err(error)
            }
        }
    }

    async fn stage_children(
        &self,
        tasks: Vec<CodingTask>,
        registered: &mut Vec<(AgentId, InvocationId)>,
    ) -> Result<Vec<StagedChild>, CodingError> {
        let record = serde_json::to_vec(&tasks).map_err(CodingError::owner)?;
        let mut staged = Vec::with_capacity(tasks.len());
        for task in tasks {
            staged.push(self.prepare_child(task, registered).await?);
        }
        for child in &staged {
            self.bind_child_scope(child).await?;
        }
        context::append_line(
            &self.records.join("tasks.jsonl"),
            &record,
            MAX_TASK_HISTORY_BYTES,
        )?;
        Ok(staged)
    }

    async fn bind_child_scope(&self, child: &StagedChild) -> Result<(), CodingError> {
        self.ledger
            .bind_runtime(self.epoch, &self.policy_digest)
            .await
            .map_err(CodingError::owner)?;
        self.ledger
            .sequencer()
            .append_child_invocation(
                &child.agent,
                &child.invocation,
                ChildInvocationRecord::ScopeBound {
                    checkout_id: child.checkout.clone(),
                    scope_sha256: child.scope.clone(),
                },
            )
            .await
            .map_err(CodingError::owner)?;
        Ok(())
    }

    /// A registration that cannot be taken back leaves a queued child no tool
    /// can schedule or clear, so the job fails rather than carry it.
    async fn unregister(&self, registered: &[(AgentId, InvocationId)]) {
        for (agent, invocation) in registered.iter().rev() {
            if let Err(error) = self
                .application
                .store
                .remove_unscheduled_coding_child(self.job_id(), agent, invocation)
                .await
            {
                self.latch_error(&CodingError::owner(error));
            }
        }
    }
}
