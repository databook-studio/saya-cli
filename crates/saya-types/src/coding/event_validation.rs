use std::num::NonZeroU64;

use super::event_kinds::{ApprovalDisposition, CodingEventKind};
use super::events::CodingEventError;
use super::{
    AgentId, CheckoutOperationState, JobId, ManagedCheckoutLifecycle, RepositoryMutationAction,
};

pub(super) fn validate_event(
    sequence: NonZeroU64,
    job_id: &JobId,
    agent_id: Option<&AgentId>,
    invocation_id: Option<&super::InvocationId>,
    event: &CodingEventKind,
) -> Result<(), CodingEventError> {
    let valid = match event {
        CodingEventKind::ChildInvocation { record } => {
            agent_id.is_some()
                && invocation_id.is_some()
                && match record.as_ref() {
                    super::ChildInvocationRecord::ScopeBound { .. } => true,
                    super::ChildInvocationRecord::FollowUpPrepared {
                        previous_invocation_id,
                        previous_completion_sequence,
                        follow_up_index,
                        ..
                    } => {
                        invocation_id != Some(previous_invocation_id)
                            && *previous_completion_sequence < sequence
                            && (1..=super::MAX_CHILD_FOLLOW_UPS).contains(follow_up_index)
                    }
                    super::ChildInvocationRecord::FollowUpAdopted {
                        prepared_sequence, ..
                    } => *prepared_sequence < sequence,
                }
        }
        CodingEventKind::TransportObserved { span } => {
            let data = span.data();
            &data.job_id == job_id
                && data.agent_id.as_ref() == agent_id
                && data.invocation_id.as_ref() == invocation_id
                && data.reservation_sequence < sequence
        }
        CodingEventKind::RunTerminated { state, failure, .. } => {
            agent_id.is_none()
                && invocation_id.is_none()
                && matches!(state, super::JobState::Failed | super::JobState::Cancelled)
                && failure.agent_id.is_some() == failure.invocation_id.is_some()
        }
        CodingEventKind::RuntimeResumed { .. } => agent_id.is_none() && invocation_id.is_none(),
        CodingEventKind::StateChanged { .. } | CodingEventKind::ResultReported { .. } => true,
        // A snapshot warning belongs to the job, not a child or invocation,
        // and names a bounded repository-relative path.
        CodingEventKind::SnapshotWarning { path, .. } => {
            agent_id.is_none()
                && invocation_id.is_none()
                && !path.is_empty()
                && path.len() <= super::MAX_SNAPSHOT_WARNING_PATH_BYTES
        }
        // A progress check belongs to the job, not a child or invocation.
        CodingEventKind::ProgressReported { .. } => agent_id.is_none() && invocation_id.is_none(),
        CodingEventKind::BudgetReserved { reservation } => {
            sequence == reservation.reservation_sequence()
                && job_id == reservation.job_id()
                && agent_id == reservation.agent_id()
                && invocation_id == reservation.invocation_id()
        }
        CodingEventKind::BudgetSettled { settlement } => {
            sequence == settlement.settlement_sequence()
                && settlement.reservation_sequence() < sequence
                && job_id == settlement.job_id()
                && agent_id.is_none()
                && invocation_id.is_none()
        }
        CodingEventKind::BudgetReconciled {
            reservation_sequence,
            ..
        } => *reservation_sequence < sequence && agent_id.is_none() && invocation_id.is_none(),
        CodingEventKind::ApprovalRequested { request } => {
            sequence == request.request_sequence()
                && job_id == request.job_id()
                && agent_id == request.agent_id()
                && invocation_id == request.invocation_id()
        }
        CodingEventKind::ApprovalClosed {
            request_sequence,
            disposition,
        } => {
            let actor_paired = agent_id.is_some() == invocation_id.is_some();
            let consumed_in_order = match disposition {
                ApprovalDisposition::Consumed {
                    reservation_sequence,
                } => *request_sequence < *reservation_sequence && *reservation_sequence < sequence,
                _ => true,
            };
            request_sequence < &sequence && actor_paired && consumed_in_order
        }
        CodingEventKind::CheckoutCreateIntent {
            checkout,
            operation,
        } => {
            sequence == operation.operation_sequence()
                && sequence == checkout.created_by_sequence()
                && job_id == checkout.job_id()
                && job_id == operation.job_id()
                && checkout.managed_checkout_id() == operation.managed_checkout_id()
                && operation.action() == RepositoryMutationAction::CreateCheckout
                && operation.state() == CheckoutOperationState::Intent
                && checkout.lifecycle() == ManagedCheckoutLifecycle::Creating
                && agent_id == checkout.agent_id()
                && invocation_id.is_none()
        }
        CodingEventKind::CheckoutOperationIntent { operation } => {
            sequence == operation.operation_sequence()
                && job_id == operation.job_id()
                && operation.action() != RepositoryMutationAction::CreateCheckout
                && operation.state() == CheckoutOperationState::Intent
                && invocation_id.is_none()
        }
        CodingEventKind::CheckoutOperationOutcome { operation } => {
            operation.operation_sequence() < sequence
                && job_id == operation.job_id()
                && operation.state() != CheckoutOperationState::Intent
                && invocation_id.is_none()
        }
    };
    if valid {
        Ok(())
    } else {
        Err(CodingEventError::InvalidEventLink)
    }
}
