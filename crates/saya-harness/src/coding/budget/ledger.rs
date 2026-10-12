//! Shared budget vocabulary: admission ceilings, ledger errors, and the
//! observable snapshot of a job's budget state.
use std::time::Duration;
use thiserror::Error;

mod owner;
mod state;

pub use owner::JobBudgetLedger;
pub use state::ChildSession;
pub(super) use state::{Counts, LocalCounts, State};

pub const MAX_LIVE_PERMITS: usize = 1_024;
pub const MAX_RESERVATIONS: u64 = 10_000;

/// Harness-side operational ceiling on concurrently active command slots;
/// outside the authenticated budget policy, like `MAX_LIVE_PERMITS`.
pub const MAX_COMMAND_SLOTS: u64 = 2;

#[derive(Debug, Clone, Copy)]
/// Finite job-wide admission ceilings for logical requests, actual sends,
/// tools, active model/child slots, and elapsed time.
pub struct CountLimits {
    /// Cumulative admitted logical receives; known no-start outcomes do not refund this count.
    pub logical_requests: u64,
    /// Cumulative admitted physical sends, reserved separately for each observed send.
    pub transport_attempts: u64,
    /// Cumulative admitted tool items, reserved once per whole tool batch.
    pub tool_calls: u64,
    /// Active model requests, retained after an uncertain outcome.
    pub model_slots: u64,
    /// Active child runs, retained after an uncertain outcome.
    pub child_slots: u64,
    /// Monotonic elapsed-time limit beginning when the fresh owner is constructed.
    pub deadline: Duration,
}

#[derive(Debug, Error)]
pub enum BudgetLedgerError {
    #[error("finite positive limits are required")]
    InvalidLimits,
    #[error("job is not fresh and running")]
    NotFreshRunning,
    #[error("store or event history failed validation")]
    Store,
    #[error("stored budget policy does not match the proposed limits")]
    PolicyMismatch,
    #[error("no budget policy is recorded for this job")]
    PolicyMissing,
    #[error("durable budget history exceeds the bounded resume scan")]
    ResumeHistoryTooLong,
    #[error("durable budget history failed validation")]
    ResumeRecordCorrupt,
}

#[derive(Debug, Clone, Copy)]
pub struct ChildCountLimits {
    pub logical_requests: u64,
    pub tool_calls: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
/// Counts and optional verified-policy token charges; never an unconditional
/// provider billing or restart-recovery guarantee.
pub struct BudgetSnapshot {
    /// Admitted counts; these do not represent measured provider/tool success.
    pub tokens_charged: Option<u64>,
    pub token_limit: Option<u64>,
    pub token_bound_violated: bool,
    pub logical_requests: u64,
    pub transport_attempts: u64,
    pub tool_calls: u64,
    pub model_slots_active: u64,
    pub child_slots_active: u64,
    pub command_slots_active: u64,
    pub tool_permits_pending: u64,
    pub unknown_logical_requests: u64,
    pub unknown_transport_attempts: u64,
    pub unknown_tool_calls: u64,
    pub measured_tool_calls: u64,
    /// Reservations with no settlement found when this owner resumed: interrupted effects whose usage is unknown.
    pub unsettled_reservations: u64,
    pub reported_input_tokens: Option<u64>,
    pub reported_output_tokens: Option<u64>,
    /// Display-only estimated usage totals accumulated from incomplete
    /// physical-attempt usage reports; never enforced, never a refund, and
    /// never a measured value. Resume (restart) does not reconstruct
    /// estimates: they are not durable. None when no incomplete usage was
    /// reported.
    pub estimated_input_tokens: Option<u64>,
    /// Output-token counterpart of [`Self::estimated_input_tokens`].
    pub estimated_output_tokens: Option<u64>,
    pub usage_coverage: UsageCoverage,
    pub poisoned: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
/// Completeness of source-reported N02 physical-attempt token observations.
pub enum UsageCoverage {
    #[default]
    Unavailable,
    Partial,
    Complete,
    Unknown,
}
