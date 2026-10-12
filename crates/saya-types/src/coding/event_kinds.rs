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
    Expired,
    Revoked,
    Cancelled,
    Consumed { reservation_sequence: NonZeroU64 },
}

/// Typed coding events. Storage owns global ordering and durable transitions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum CodingEventKind {
    TransportObserved {
        span: Box<super::TransportSpanV1>,
    },
    StateChanged {
        state: JobState,
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
    TransportObserved {
        span: Box<super::TransportSpanV1>,
    },
    StateChanged {
        state: JobState,
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
            Self::TransportObserved { span } => CodingEventKind::TransportObserved { span },
            Self::StateChanged { state } => CodingEventKind::StateChanged { state },
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
