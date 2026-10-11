//! Durable native coding metadata, bounded joined results, and control intents.

use saya_types::{CheckoutOperationState, ManagedCheckoutLifecycle, RepositoryMutationAction};

mod approval_consumption;
mod approval_records;
mod approvals;
mod budget_records;
mod budgets;
mod checkout_event;
mod checkout_operations;
mod checkout_plan;
mod checkout_reads;
mod checkouts;
mod events;
mod policies;
mod projection;
mod records;
mod replay;
pub(crate) mod schema;

pub use approval_consumption::{CodingApprovalConsumptionAppend, CodingApprovalConsumptionReceipt};
pub use approval_records::{
    CodingApprovalClosure, CodingApprovalRequestPage, CodingApprovalRequestRecord,
};
pub use approvals::CodingApprovalConsumptionContext;
pub use budget_records::{CodingBudgetReservationPage, CodingBudgetReservationRecord};
pub use checkout_operations::MAX_CHECKOUT_OPERATION_INTENTS_PER_JOB;
pub use checkout_plan::{CheckoutPlanAppend, MAX_CHECKOUT_PLAN_BYTES, StoredCheckoutPlan};
pub use checkout_reads::{CheckoutOperationPage, MAX_CHECKOUT_OPERATIONS_PER_JOB};
pub use checkouts::{MAX_CHECKOUT_RECORDS_PER_JOB, ManagedCheckoutPage, ManagedCheckoutRecord};
pub use events::{CodingEventAppend, MAX_CODING_EVENT_PAGE_SIZE};
pub use policies::{CodingBudgetPolicyAppend, StoredBudgetPolicy};
pub use records::{
    CodingChildPage, CodingChildRecord, CodingJobRecord, NewCodingChild, NewCodingJob,
};
pub use replay::CodingEventPage;

pub(crate) fn lifecycle(
    action: RepositoryMutationAction,
    state: CheckoutOperationState,
) -> ManagedCheckoutLifecycle {
    match (action, state) {
        (RepositoryMutationAction::CreateCheckout, CheckoutOperationState::Intent) => {
            ManagedCheckoutLifecycle::Creating
        }
        (RepositoryMutationAction::CreateCheckout, CheckoutOperationState::Completed) => {
            ManagedCheckoutLifecycle::Active
        }
        (RepositoryMutationAction::CreateCheckout, CheckoutOperationState::Failed) => {
            ManagedCheckoutLifecycle::Failed
        }
        (RepositoryMutationAction::RemoveCheckout, CheckoutOperationState::Intent) => {
            ManagedCheckoutLifecycle::Removing
        }
        (RepositoryMutationAction::RemoveCheckout, CheckoutOperationState::Completed) => {
            ManagedCheckoutLifecycle::Removed
        }
        (RepositoryMutationAction::RemoveCheckout, CheckoutOperationState::Failed) => {
            ManagedCheckoutLifecycle::Retained
        }
        (_, CheckoutOperationState::Uncertain) => ManagedCheckoutLifecycle::Uncertain,
        (RepositoryMutationAction::IntegrateCandidate | RepositoryMutationAction::ApplyBack, _) => {
            ManagedCheckoutLifecycle::Retained
        }
        _ => ManagedCheckoutLifecycle::Uncertain,
    }
}

mod result_reads;
mod results;
pub use result_reads::{CodingCompletion, CodingCompletionPage, MAX_COMPLETION_PAGE_SIZE};
pub use results::MAX_COMPLETIONS_PER_JOB;

mod recovery;
mod runtime;
pub use recovery::CodingRecoveryStatus;
pub use runtime::CodingRuntimeAuthority;

mod job_reads;
pub use job_reads::CodingJobPage;

mod control_requests;
