use std::num::NonZeroU64;

use serde::{Deserialize, Serialize};

use super::events::CodingEventError;
use super::{
    ApprovalRequestV1, BudgetReservationV1, BudgetSettlementV1, CheckoutOperationV1,
    CodingResultOutcome, JobState, ManagedCheckoutRecordV1, Sha256Digest,
};

/// Audit disposition for one pending request; it is never a reusable grant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "disposition", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum ApprovalDisposition {
    Denied,
    /// A human granted this exact request; it is an audit fact, never a
    /// reusable grant.
    Granted,
    Expired,
    Revoked,
    Cancelled,
    Consumed {
        reservation_sequence: NonZeroU64,
    },
}

/// The longest repository-relative path a snapshot warning may name. It
/// mirrors the snapshot path bound, so an event can never carry a longer
/// path than a manifest could.
pub const MAX_SNAPSHOT_WARNING_PATH_BYTES: usize = 512;

/// Why a non-fatal snapshot warning was recorded. The named path is in the
/// snapshot; the warning states why it is worth an operator's attention.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SnapshotWarningReason {
    /// A tracked credential-shaped path was included as repository content.
    SensitiveTrackedIncluded,
}

/// Typed coding events. Storage owns global ordering and durable transitions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum CodingEventKind {
    ChildInvocation {
        record: Box<super::ChildInvocationRecord>,
    },
    TransportObserved {
        span: Box<super::TransportSpanV1>,
    },
    StateChanged {
        state: JobState,
    },
    /// One non-fatal snapshot fact. The path is in the snapshot; its content
    /// is never carried here.
    SnapshotWarning {
        path: String,
        reason: SnapshotWarningReason,
    },
    /// One turn's check against the no-progress guard, recorded by the root
    /// loop. `reset` is a turn that made progress after a non-zero streak;
    /// otherwise `turns_without_progress` is the streak after this turn.
    ProgressReported {
        turn: u64,
        turns_without_progress: u32,
        limit: Option<u32>,
        reset: bool,
        last_progress_turn: Option<u64>,
        message: String,
    },
    RunTerminated {
        state: JobState,
        failure: super::CodingRunFailure,
        secondary: Option<super::CodingFailureClass>,
        diagnostic_fault: Option<super::CodingDiagnosticFault>,
    },
    RuntimeResumed {
        revocation_epoch: NonZeroU64,
        policy_sha256: Sha256Digest,
    },
    ResultReported {
        source_sha256: Sha256Digest,
        outcome: CodingResultOutcome,
    },
    BudgetReserved {
        reservation: Box<BudgetReservationV1>,
    },
    BudgetSettled {
        settlement: Box<BudgetSettlementV1>,
    },
    BudgetReconciled {
        reservation_sequence: NonZeroU64,
        note_sha256: Sha256Digest,
    },
    ApprovalRequested {
        request: Box<ApprovalRequestV1>,
    },
    ApprovalClosed {
        request_sequence: NonZeroU64,
        disposition: ApprovalDisposition,
    },
    CheckoutCreateIntent {
        checkout: Box<ManagedCheckoutRecordV1>,
        operation: Box<CheckoutOperationV1>,
    },
    CheckoutOperationIntent {
        operation: Box<CheckoutOperationV1>,
    },
    CheckoutOperationOutcome {
        operation: Box<CheckoutOperationV1>,
    },
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum WireCodingEventKind {
    ChildInvocation {
        record: Box<super::ChildInvocationRecord>,
    },
    TransportObserved {
        span: Box<super::TransportSpanV1>,
    },
    StateChanged {
        state: JobState,
    },
    SnapshotWarning {
        path: String,
        reason: SnapshotWarningReason,
    },
    ProgressReported {
        turn: u64,
        turns_without_progress: u32,
        limit: Option<u32>,
        reset: bool,
        last_progress_turn: Option<u64>,
        message: String,
    },
    RunTerminated {
        state: JobState,
        failure: super::CodingRunFailure,
        secondary: Option<super::CodingFailureClass>,
        diagnostic_fault: Option<super::CodingDiagnosticFault>,
    },
    RuntimeResumed {
        revocation_epoch: NonZeroU64,
        policy_sha256: String,
    },
    ResultReported {
        source_sha256: String,
        outcome: CodingResultOutcome,
    },
    BudgetReserved {
        reservation: Box<BudgetReservationV1>,
    },
    BudgetSettled {
        settlement: Box<BudgetSettlementV1>,
    },
    BudgetReconciled {
        reservation_sequence: NonZeroU64,
        note_sha256: String,
    },
    ApprovalRequested {
        request: Box<ApprovalRequestV1>,
    },
    ApprovalClosed {
        request_sequence: NonZeroU64,
        disposition: ApprovalDisposition,
    },
    CheckoutCreateIntent {
        checkout: Box<ManagedCheckoutRecordV1>,
        operation: Box<CheckoutOperationV1>,
    },
    CheckoutOperationIntent {
        operation: Box<CheckoutOperationV1>,
    },
    CheckoutOperationOutcome {
        operation: Box<CheckoutOperationV1>,
    },
}

impl WireCodingEventKind {
    pub(super) fn into_public(self) -> Result<CodingEventKind, CodingEventError> {
        Ok(match self {
            Self::ChildInvocation { record } => CodingEventKind::ChildInvocation { record },
            Self::TransportObserved { span } => CodingEventKind::TransportObserved { span },
            Self::StateChanged { state } => CodingEventKind::StateChanged { state },
            Self::SnapshotWarning { path, reason } => {
                CodingEventKind::SnapshotWarning { path, reason }
            }
            Self::ProgressReported {
                turn,
                turns_without_progress,
                limit,
                reset,
                last_progress_turn,
                message,
            } => CodingEventKind::ProgressReported {
                turn,
                turns_without_progress,
                limit,
                reset,
                last_progress_turn,
                message,
            },
            Self::RunTerminated {
                state,
                failure,
                secondary,
                diagnostic_fault,
            } => CodingEventKind::RunTerminated {
                state,
                failure,
                secondary,
                diagnostic_fault,
            },
            Self::RuntimeResumed {
                revocation_epoch,
                policy_sha256,
            } => CodingEventKind::RuntimeResumed {
                revocation_epoch,
                policy_sha256: Sha256Digest::parse(&policy_sha256)
                    .map_err(|_| CodingEventError::InvalidDigest)?,
            },
            Self::ResultReported {
                source_sha256,
                outcome,
            } => CodingEventKind::ResultReported {
                source_sha256: Sha256Digest::parse(&source_sha256)
                    .map_err(|_| CodingEventError::InvalidDigest)?,
                outcome,
            },
            Self::BudgetReserved { reservation } => CodingEventKind::BudgetReserved { reservation },
            Self::BudgetSettled { settlement } => CodingEventKind::BudgetSettled { settlement },
            Self::BudgetReconciled {
                reservation_sequence,
                note_sha256,
            } => CodingEventKind::BudgetReconciled {
                reservation_sequence,
                note_sha256: Sha256Digest::parse(&note_sha256)
                    .map_err(|_| CodingEventError::InvalidDigest)?,
            },
            Self::ApprovalRequested { request } => CodingEventKind::ApprovalRequested { request },
            Self::ApprovalClosed {
                request_sequence,
                disposition,
            } => CodingEventKind::ApprovalClosed {
                request_sequence,
                disposition,
            },
            Self::CheckoutCreateIntent {
                checkout,
                operation,
            } => CodingEventKind::CheckoutCreateIntent {
                checkout,
                operation,
            },
            Self::CheckoutOperationIntent { operation } => {
                CodingEventKind::CheckoutOperationIntent { operation }
            }
            Self::CheckoutOperationOutcome { operation } => {
                CodingEventKind::CheckoutOperationOutcome { operation }
            }
        })
    }
}
