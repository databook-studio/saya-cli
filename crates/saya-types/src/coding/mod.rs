//! Shared identity and lifecycle contracts for native coding jobs.

mod approval_job_scope;
mod approval_scope;
mod approvals;
mod budget;
mod budget_mode;
mod budget_policy;
mod budget_policy_record;
mod budget_policy_v2;
mod budget_settlement;
mod budget_values;
mod checkout_operation;
mod checkout_values;
mod checkouts;
mod child_invocation;
mod control_wire;
mod digest;
mod event_kinds;
mod event_validation;
mod events;
mod ids;
mod jobs;
mod receipts;

pub use approval_scope::{
    ApprovalNetworkDestination, ApprovalNetworkScheme, ApprovalScope, ApprovalScopeError,
    JobApprovalAction, RepositoryMutationAction, WorkspacePath,
};
pub use approvals::{
    APPROVAL_REQUEST_VERSION, ApprovalRequestContext, ApprovalRequestError, ApprovalRequestV1,
    MAX_APPROVAL_REQUEST_BYTES,
};
pub use budget::{BUDGET_RESERVATION_VERSION, BudgetError, BudgetReservationV1};
pub use budget_mode::CodingBudgetMode;
pub use budget_policy::{BUDGET_POLICY_VERSION, BudgetLimitPolicyV1, BudgetPolicyError};
pub use budget_policy_record::CodingBudgetPolicy;
pub use budget_policy_v2::{BUDGET_POLICY_V2_VERSION, BudgetLimitPolicyV2};
pub use budget_settlement::{BUDGET_SETTLEMENT_VERSION, BudgetEffectState, BudgetSettlementV1};
pub use budget_values::{
    BudgetCharge, BudgetDimension, BudgetObservation, BudgetObservationValue,
    MAX_BUDGET_ENDPOINT_BYTES, MAX_BUDGET_ENTRIES,
};
pub use checkout_operation::{
    CHECKOUT_OPERATION_VERSION, CheckoutOperationState, CheckoutOperationV1,
};
pub use checkout_values::{
    CheckoutValueError, GitObjectId, MAX_MANAGED_CHECKOUT_PATH_BYTES, ManagedCheckoutLifecycle,
    ManagedCheckoutPath,
};
pub use checkouts::{
    CheckoutDtoError, MANAGED_CHECKOUT_RECORD_VERSION, MAX_CHECKOUT_RECORD_BYTES,
    ManagedCheckoutRecordData, ManagedCheckoutRecordV1,
};
pub use child_invocation::{ChildInvocationRecord, MAX_CHILD_FOLLOW_UPS};
pub use digest::{DigestError, Sha256Digest, sha256_hex};
pub use events::{
    ApprovalDisposition, CODING_EVENT_VERSION, CodingEventError, CodingEventKind, CodingEventV1,
    MAX_CODING_EVENT_BYTES, MAX_SNAPSHOT_WARNING_PATH_BYTES, SnapshotWarningReason,
};
pub use ids::{AgentId, InvocationId, JobId, ManagedCheckoutId, RepositoryId, SnapshotId};
pub use jobs::{JobState, JobStateTransitionError};
pub use receipts::{
    CodingResultOutcome, MAX_RECEIPT_PATH_BYTES, MAX_RECEIPT_PATH_SET_BYTES, MAX_RECEIPT_PATHS,
    MAX_RECEIPT_REFERENCE_BYTES, MAX_RESULT_RECEIPT_BYTES, RESULT_RECEIPT_VERSION, ReceiptError,
    ResultReceiptV1,
};

use thiserror::Error;

/// Why a coding identifier failed its safe wire shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum CodingIdError {
    #[error("{0} must be 1-128 ASCII letters, digits, hyphens, or underscores")]
    InvalidId(&'static str),
}

mod failure_class;
mod joined_result;
pub use failure_class::{CodingDiagnosticFault, CodingFailureClass, CodingRunFailure};
pub use joined_result::{
    JoinedResultError, JoinedResultV1, MAX_JOINED_ANSWER_BYTES, MAX_JOINED_RESULT_BYTES,
};

mod thinking;
pub use thinking::{MAX_THINKING_BUDGET_TOKENS, ThinkingEffort, ThinkingMode, ThinkingSpanData};

mod transport;
pub use transport::{
    MAX_TRANSPORT_SPAN_BYTES, TransportCallKind, TransportOutcome, TransportSpanData,
    TransportSpanError, TransportSpanV1,
};

mod token_policy;
pub use token_policy::{TokenBudgetError, TokenBudgetPolicyData, TokenBudgetPolicyV1};
