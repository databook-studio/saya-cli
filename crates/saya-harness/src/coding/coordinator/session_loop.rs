//! The running half of a schedule session: the lease, the growing child set,
//! and the loop that starts, joins and reports them. See `session.rs`.
use super::{
    admit::{self, NodeState},
    api::ChildRunError,
    events::ChildEvent,
    schedule_types::{
        ChildOutcome, ScheduleChildError, ScheduleError, ScheduleOutcome, ScheduleReport,
        ScheduleStop,
    },
    scheduled_child,
    session::{Session, SessionChild},
};
use saya_agent::{AgentError, AgentOutput, BudgetDimension, CancellationReason};
use tokio::{
    task::{Id, JoinError},
    time::Instant,
};

/// One finished child as the join set reports it: its task id and its outcome.
type ChildJoin = Result<(Id, Result<AgentOutput, ChildRunError>), JoinError>;

/// One step of the session loop.
enum Step {
    /// A submission arrived, or every submit side was dropped.
    Submitted(Option<Box<SessionChild>>),
    /// A child finished, or the join set is empty.
    Joined(Option<Box<ChildJoin>>),
    /// A session-wide cancellation fired; the next pass records why.
    Cancelled,
}

impl Session {
    pub(super) async fn run(mut self) -> Result<ScheduleReport, ScheduleError> {
        loop {
            self.check_stop();
            if self.stop.is_none() {
                self.start_admissible();
            }
            if self.children.is_empty() && self.closed {
                if self.stop.is_none() && self.states.contains(&NodeState::Pending) {
                    self.stop = Some(ScheduleStop::DependencyBlocked);
                }
                break;
            }
            let step = tokio::select! {
                biased;
                _ = self.owner.cancelled(), if self.stop.is_none() => {
                    self.cancellation.cancel_with_reason(
                        self.owner.reason().unwrap_or(CancellationReason::OwnerFailure));
                    Step::Cancelled
                }
                _ = self.caller.cancelled(), if self.stop.is_none() => {
                    self.cancellation.cancel_with_reason(
                        self.caller.reason().unwrap_or(CancellationReason::User));
                    Step::Cancelled
                }
                _ = wait_for_deadline(self.deadline), if self.stop.is_none() => {
                    self.cancellation.cancel_with_reason(CancellationReason::Budget(
                        BudgetDimension::ElapsedTime));
                    Step::Cancelled
                }
                _ = self.cancellation.cancelled(), if self.stop.is_none() => Step::Cancelled,
                received = self.receiver.recv(), if !self.closed => {
                    Step::Submitted(received.map(Box::new))
                }
                joined = self.children.join_next_with_id(), if !self.children.is_empty() => {
                    Step::Joined(joined.map(Box::new))
                }
            };
            match step {
                Step::Submitted(None) => self.closed = true,
                Step::Submitted(Some(child)) => self.admit(*child),
                Step::Cancelled => {}
                Step::Joined(None) => {}
                Step::Joined(Some(joined)) => self.finish_child(*joined).await,
            }
        }
        // Returning from every future is necessary; unsettled slot charges still
        // prevent successful release, including a failed finish_child operation.
        let final_budget = self.ledger.snapshot().await;
        if self.settled
            && final_budget.child_slots_active == 0
            && final_budget.command_slots_active == 0
        {
            if let Some(lease) = self.lease.take() {
                lease.complete();
            }
        } else {
            self.stop = Some(ScheduleStop::Unsettled);
        }
        // Preserve a stronger owner failure that latched while children were joining.
        if (final_budget.poisoned || self.owner.reason() == Some(CancellationReason::OwnerFailure))
            && self.stop != Some(ScheduleStop::Unsettled)
        {
            self.stop = Some(ScheduleStop::OwnerFailure);
        }
        // Unknown transport-attempt usage never reaches here: only a child with
        // admitted work or an unknown effect reports started.
        let mut child_unsettled = false;
        for plan in &self.plans {
            if self
                .ledger
                .child_started(&plan.agent, &plan.invocation)
                .await
            {
                child_unsettled = true;
                break;
            }
        }
        let effects_uncertain = child_unsettled || self.stop == Some(ScheduleStop::Unsettled);
        Ok(ScheduleReport {
            effects_uncertain,
            outcomes: self.outcomes,
            stop: self.stop,
            not_started: self
                .plans
                .iter()
                .zip(self.states)
                .filter(|(_, state)| *state == NodeState::Pending)
                .map(|(plan, _)| plan.agent.clone())
                .collect(),
        })
    }

    /// Records the session-wide stop the owner, the caller or the deadline asks
    /// for. A stop closes the session to further children.
    fn check_stop(&mut self) {
        if self.stop.is_some() {
            return;
        }
        let reason = if self.owner.is_cancelled() {
            self.owner.reason()
        } else if self.caller.is_cancelled() {
            self.caller.reason()
        } else if self
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            Some(CancellationReason::Budget(BudgetDimension::ElapsedTime))
        } else {
            self.cancellation.reason()
        };
        let Some(reason) = reason else { return };
        // An owner failure is never reported as a child's own failure.
        self.stop = Some(if reason == CancellationReason::OwnerFailure {
            ScheduleStop::OwnerFailure
        } else {
            ScheduleStop::Cancelled(reason)
        });
        self.cancellation.cancel_with_reason(reason);
        self.close();
    }

    fn close(&mut self) {
        self.closed = true;
        self.receiver.close();
    }

    /// Starts every pending child the slot bound allows. A child over the bound
    /// waits here; it is never refused.
    fn start_admissible(&mut self) {
        for index in 0..self.nodes.len() {
            if !admit::can_start(index, &self.plans, &self.states, self.limits) {
                continue;
            }
            let Some(node) = self.nodes[index].take() else {
                continue;
            };
            if let Some(lease) = self.lease.as_mut() {
                lease.mark_started();
            }
            self.states[index] = NodeState::Running;
            let handle = self.children.spawn(scheduled_child::run(
                node,
                self.ledger.clone(),
                self.cancellation.clone(),
                self.progress.clone(),
            ));
            self.active.insert(handle.id(), index);
        }
    }

    fn admit(&mut self, child: SessionChild) {
        self.states.push(NodeState::Pending);
        self.waiters.push(Some(child.waiter));
        self.nodes.push(Some(child.node));
        self.plans.push(child.plan);
    }

    async fn finish_child(
        &mut self,
        joined: Result<(Id, Result<AgentOutput, ChildRunError>), JoinError>,
    ) {
        let (id, result) = match joined {
            Ok((id, result)) => (id, result.map_err(ScheduleChildError::Run)),
            Err(error) => {
                self.settled = false;
                (error.id(), Err(ScheduleChildError::Join(error)))
            }
        };
        let Some(index) = self.active.remove(&id) else {
            self.settled = false;
            self.stop = Some(ScheduleStop::Unsettled);
            self.cancellation
                .cancel_with_reason(CancellationReason::OwnerFailure);
            return;
        };
        let completion = super::results::persist(&self.ledger, &self.plans[index], &result).await;
        let result = match result {
            Ok(output) if output.truncated => {
                Err(if super::failure_class::budget_stopped(&output) {
                    ScheduleChildError::BudgetStopped
                } else {
                    ScheduleChildError::Incomplete
                })
            }
            other => other,
        };
        self.states[index] = if result.is_ok() {
            NodeState::Succeeded
        } else {
            NodeState::Failed
        };
        if completion.is_err() {
            self.settled = false;
            self.stop = Some(ScheduleStop::OwnerFailure);
            self.cancellation
                .cancel_with_reason(CancellationReason::OwnerFailure);
            self.close();
        }
        if matches!(
            &result,
            Err(ScheduleChildError::Run(ChildRunError::Unsettled))
        ) {
            self.settled = false;
        }
        // A child's own failure is its own: the siblings keep running. Only an
        // owner failure stops the whole session.
        if let Some(reason) = owner_failure(&result) {
            self.stop = Some(ScheduleStop::OwnerFailure);
            self.cancellation.cancel_with_reason(reason);
            self.close();
        }
        let (completion_sequence, completion_error) = match completion {
            Ok(sequence) => {
                // Completion events are best-effort notifications only. The
                // durable receipt was appended first and is replayed by the
                // coordinator if this bounded channel is full or closed.
                let _ = self.progress.try_send(ChildEvent::completion(
                    self.ledger.job_id().await,
                    self.plans[index].agent.clone(),
                    self.plans[index].invocation.clone(),
                    sequence,
                ));
                (Some(sequence), None)
            }
            Err(error) => (None, Some(error)),
        };
        let outcome = ScheduleOutcome {
            agent_id: self.plans[index].agent.clone(),
            invocation_id: self.plans[index].invocation.clone(),
            result,
            completion_sequence,
            completion_error,
        };
        if let Some(waiter) = self.waiters[index].take() {
            let _ = waiter.send(ChildOutcome {
                agent_id: outcome.agent_id.clone(),
                invocation_id: outcome.invocation_id.clone(),
                succeeded: outcome.result.is_ok(),
                sequence: outcome.completion_sequence,
            });
        }
        self.outcomes.push(outcome);
    }
}

/// The session-wide stop one child's own outcome can ask for. Anything else is
/// that child's own failure, and its siblings keep running.
fn owner_failure(result: &Result<AgentOutput, ScheduleChildError>) -> Option<CancellationReason> {
    match result {
        Err(ScheduleChildError::Run(
            ChildRunError::OwnerFailure
            | ChildRunError::Cancelled {
                reason: CancellationReason::OwnerFailure,
            }
            | ChildRunError::Agent(AgentError::EffectOwnerFailure(_)),
        )) => Some(CancellationReason::OwnerFailure),
        _ => None,
    }
}

async fn wait_for_deadline(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}
