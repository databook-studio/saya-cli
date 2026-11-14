use super::{ChildFollowUpRequest, ChildFollowUpStarted};
use crate::commands::coding::{CodingError, NativeCodingJob, create::digest, task_history};
use saya_harness::coding::{
    budget::{BudgetSnapshot, ChildContinuationError},
    coordinator::ScheduledChild,
    integration::{DEFAULT_DELTA_DEADLINE, EvidenceBinding},
    worktree::verify_owned_checkout,
};
use saya_types::{
    ChildInvocationRecord, CodingEventKind, CodingResultOutcome, InvocationId, JobState,
};

/// Live budget state that refuses a follow-up outright. Unknown usage of a
/// model request or its transport attempts is not among it: a model request
/// cannot have changed the workspace the follow-up reuses.
fn blocks_follow_up(budget: &BudgetSnapshot) -> bool {
    budget.poisoned || budget.child_slots_active != 0 || budget.command_slots_active != 0
}

pub(super) struct Prepared {
    pub(super) node: ScheduledChild,
    pub(super) receipt: ChildFollowUpStarted,
}
impl NativeCodingJob {
    pub(super) async fn prepare_follow_up(
        &self,
        request: ChildFollowUpRequest,
    ) -> Result<Prepared, CodingError> {
        if request.instruction.trim().is_empty()
            || request.instruction.len() > 4096
            || request
                .instruction
                .chars()
                .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\t'))
        {
            return Err(CodingError::InvalidRequest);
        }
        let instruction = saya_types::redact(&request.instruction);
        if instruction.len() > 4096 || self.cancellation.is_cancelled() {
            return Err(CodingError::InvalidRequest);
        }
        let budget = self.ledger.snapshot().await;
        if blocks_follow_up(&budget) {
            return Err(CodingError::NotOwned);
        }
        let children = self
            .application
            .store
            .list_coding_children(self.job_id(), None, 100)
            .await
            .map_err(CodingError::owner)?;
        if children.next_cursor.is_some() {
            return Err(CodingError::InvalidRequest);
        }
        let child = children
            .items
            .iter()
            .find(|child| child.agent_id == request.agent_id)
            .ok_or(CodingError::NotOwned)?;
        let history = self
            .application
            .store
            .coding_child_invocation_history(self.job_id(), &request.agent_id)
            .await
            .map_err(CodingError::owner)?;
        if (budget.unsettled_reservations != 0 || budget.unknown_tool_calls != 0)
            && !super::retained_root::allows(self, &budget).await?
        {
            return Err(CodingError::NotOwned);
        }
        let (checkout, scope) = history
            .iter()
            .find_map(|event| match event.event() {
                CodingEventKind::ChildInvocation { record } => match record.as_ref() {
                    ChildInvocationRecord::ScopeBound {
                        checkout_id,
                        scope_sha256,
                    } => Some((checkout_id.clone(), scope_sha256.clone())),
                    _ => None,
                },
                _ => None,
            })
            .ok_or(CodingError::NotOwned)?;
        let (tasks, _) = task_history::read_tasks(&self.records)?;
        let task = tasks
            .get(&request.agent_id)
            .cloned()
            .ok_or(CodingError::NotOwned)?;
        // The follow-up runs in W, the same working checkout as every other
        // helper: the recorded checkout is the job's W, verified at use
        // time, and there is no per-child lease to hold or release.
        let verified = verify_owned_checkout(&self.ownership(&checkout)?, &self.cancellation)
            .await
            .map_err(CodingError::owner)?;
        let ignored = self.checkout_ignored_paths(&verified).await?;
        let source = EvidenceBinding::bind(
            &verified,
            &ignored,
            &self.cancellation,
            DEFAULT_DELTA_DEADLINE,
        )
        .map_err(CodingError::owner)?
        .source_sha256()
        .clone();
        let instruction_sha256 = digest(instruction.as_bytes())?;
        let (invocation, previous, record) = if child.state == JobState::Completed {
            let completion = self
                .application
                .store
                .get_coding_completion(self.job_id(), request.expected_completion_sequence)
                .await
                .map_err(CodingError::owner)?
                .ok_or(CodingError::NotOwned)?;
            if completion.result.agent_id() != &request.agent_id
                || completion.result.invocation_id() != &child.current_invocation_id
                || completion.result.outcome() != CodingResultOutcome::Completed
            {
                return Err(CodingError::NotOwned);
            }
            let count=history.iter().filter(|event|matches!(event.event(),CodingEventKind::ChildInvocation{record} if matches!(record.as_ref(),ChildInvocationRecord::FollowUpPrepared{..}))).count();
            if count >= 3 {
                return Err(CodingError::InvalidRequest);
            }
            let identity = digest(
                &serde_json::to_vec(&serde_json::json!([
                    self.job_id(),
                    request.agent_id,
                    child.current_invocation_id,
                    request.expected_completion_sequence,
                    self.epoch,
                    count + 1,
                    instruction_sha256,
                    source
                ]))
                .map_err(CodingError::owner)?,
            )?;
            let invocation = InvocationId::parse(&format!("inv-follow-{}", identity.as_str()))
                .map_err(CodingError::owner)?;
            let record = ChildInvocationRecord::FollowUpPrepared {
                checkout_id: checkout.clone(),
                previous_invocation_id: child.current_invocation_id.clone(),
                previous_completion_sequence: request.expected_completion_sequence,
                source_sha256: source.clone(),
                scope_sha256: scope.clone(),
                instruction_sha256: instruction_sha256.clone(),
                follow_up_index: (count + 1) as u8,
            };
            (invocation, child.current_invocation_id.clone(), record)
        } else if child.state == JobState::Queued {
            let prepared=history.iter().find(|event|event.invocation_id()==Some(&child.current_invocation_id) && matches!(event.event(),CodingEventKind::ChildInvocation{record} if matches!(record.as_ref(),ChildInvocationRecord::FollowUpPrepared{..}))).ok_or(CodingError::NotOwned)?;
            let CodingEventKind::ChildInvocation { record } = prepared.event() else {
                return Err(CodingError::NotOwned);
            };
            let ChildInvocationRecord::FollowUpPrepared {
                previous_invocation_id,
                previous_completion_sequence,
                source_sha256: old_source,
                instruction_sha256: old_instruction,
                ..
            } = record.as_ref()
            else {
                return Err(CodingError::NotOwned);
            };
            if *previous_completion_sequence != request.expected_completion_sequence
                || old_source != &source
                || old_instruction != &instruction_sha256
            {
                return Err(CodingError::NotOwned);
            }
            (
                child.current_invocation_id.clone(),
                previous_invocation_id.clone(),
                ChildInvocationRecord::FollowUpAdopted {
                    prepared_sequence: prepared.sequence(),
                    source_sha256: source.clone(),
                    instruction_sha256: instruction_sha256.clone(),
                },
            )
        } else {
            return Err(CodingError::NotOwned);
        };
        let (node, _) = self
            .compose_owned_child(
                task,
                checkout.clone(),
                invocation.clone(),
                Some(&instruction),
                Some(&scope),
                Some((&previous, request.expected_completion_sequence, &source)),
            )
            .await?;
        self.ledger
            .sequencer()
            .append_child_invocation(&request.agent_id, &invocation, record)
            .await
            .map_err(|error| match error {
                ChildContinuationError::Refused => CodingError::NotOwned,
                other => CodingError::owner(other),
            })?;
        Ok(Prepared {
            node,
            receipt: ChildFollowUpStarted {
                agent_id: request.agent_id,
                previous_invocation_id: previous,
                invocation_id: invocation,
                previous_completion_sequence: request.expected_completion_sequence,
                checkout_id: checkout,
            },
        })
    }
}

#[cfg(all(test, unix))]
#[path = "preparation/gate_tests.rs"]
mod gate_tests;
