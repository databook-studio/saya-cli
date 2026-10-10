use super::admission::StoredPermit;
use saya_agent::{
    CancellationReason, CancellationToken, EffectAdmissionError, EffectOwnerFailure, EffectPermit,
};
use saya_store::{CodingBudgetPolicyAppend, SqliteStateStore, StoreError};
use saya_types::{AgentId, BudgetDimension, InvocationId, JobId};
use std::{
    collections::HashMap,
    num::NonZeroU64,
    sync::Arc,
    time::{Duration, Instant},
};
use thiserror::Error;
use tokio::sync::Mutex;

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
/// In-memory admitted counts and observed outcomes; not a token, spend, byte,
/// or restart-recovery guarantee.
pub struct BudgetSnapshot {
    /// Admitted counts; these do not represent measured provider/tool success.
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

#[derive(Clone)]
pub struct JobBudgetLedger {
    pub(super) inner: Arc<Mutex<State>>,
    pub(super) cancellation: CancellationToken,
}

pub(super) struct State {
    pub(super) store: SqliteStateStore,
    pub(super) job: JobId,
    pub(super) limits: CountLimits,
    pub(super) started: Instant,
    pub(super) sequence: u64,
    pub(super) next_id: u64,
    pub(super) reservations: u64,
    pub(super) counts: Counts,
    pub(super) permits: HashMap<u64, StoredPermit>,
    pub(super) groups: HashMap<NonZeroU64, super::tools::ToolGroup>,
    pub(super) local: HashMap<(AgentId, InvocationId), LocalCounts>,
    pub(super) cancellation: CancellationToken,
    pub(super) snapshot: BudgetSnapshot,
    /// One usage observation was unknown or lacked complete reported totals;
    /// usage coverage may never read Complete again.
    pub(super) usage_ever_unknown: bool,
    pub(super) usage_ever_incomplete: bool,
}

#[derive(Default)]
pub(super) struct Counts {
    pub(super) logical: u64,
    pub(super) transport: u64,
    pub(super) tools: u64,
    pub(super) model_slots: u64,
    pub(super) child_slots: u64,
    pub(super) command_slots: u64,
}

#[derive(Default)]
pub(super) struct LocalCounts {
    pub(super) logical: u64,
    pub(super) tools: u64,
    pub(super) limits: Option<ChildCountLimits>,
}

impl JobBudgetLedger {
    /// Builds a fresh owner. The supplied limits are converted to the job's
    /// durable policy identity with a checked deadline conversion and
    /// recorded before the fresh-job guards; an already-recorded policy
    /// must match exactly or construction is refused with the recorded row
    /// untouched. A sub-millisecond-only or overflowing deadline, or any
    /// zero limit, is refused as invalid limits.
    pub async fn new(
        store: SqliteStateStore,
        job: JobId,
        limits: CountLimits,
    ) -> Result<Self, BudgetLedgerError> {
        if [
            limits.logical_requests,
            limits.transport_attempts,
            limits.tool_calls,
            limits.model_slots,
            limits.child_slots,
        ]
        .contains(&0)
            || limits.deadline.is_zero()
        {
            return Err(BudgetLedgerError::InvalidLimits);
        }
        let policy = super::fresh::proposed_policy(&limits)?;
        match store.record_coding_budget_policy(&job, &policy).await {
            Ok(
                CodingBudgetPolicyAppend::Recorded | CodingBudgetPolicyAppend::AlreadyPresentSame,
            ) => {}
            Err(StoreError::PolicyConflict) => return Err(BudgetLedgerError::PolicyMismatch),
            Err(_) => return Err(BudgetLedgerError::Store),
        }
        let started = Instant::now();
        let sequence = super::fresh::initial_sequence(&store, &job).await?;
        let cancellation = CancellationToken::new();
        Ok(Self {
            inner: Arc::new(Mutex::new(State {
                store,
                job,
                limits,
                started,
                sequence,
                next_id: 1,
                reservations: 0,
                counts: Counts::default(),
                permits: HashMap::new(),
                groups: HashMap::new(),
                local: HashMap::new(),
                cancellation: cancellation.clone(),
                snapshot: BudgetSnapshot::default(),
                usage_ever_unknown: false,
                usage_ever_incomplete: false,
            })),
            cancellation,
        })
    }

    /// Rebuilds a resumed owner from durable history. The proposed limits
    /// only authenticate the recorded policy — the resumed ceilings come
    /// from the stored policy, never from this process's configuration —
    /// and the counts, unknown counters, consumed slot charges, and next
    /// event sequence are folded from the durable records. The deadline
    /// length is policy-authenticated, but elapsed time is NOT durably
    /// recorded: the resumed owner restarts its deadline clock and does
    /// not claim elapsed continuity. A job with history but no recorded
    /// policy is never adopted.
    pub async fn resume(
        store: SqliteStateStore,
        job: JobId,
        limits: CountLimits,
    ) -> Result<Self, BudgetLedgerError> {
        super::resume::resume_ledger(store, job, limits).await
    }

    pub fn root_view(&self) -> super::RootBudgetView {
        super::RootBudgetView {
            inner: self.inner.clone(),
        }
    }

    pub(super) fn child(&self, agent: AgentId, invocation: InvocationId) -> super::ChildBudgetView {
        super::ChildBudgetView {
            inner: self.inner.clone(),
            agent,
            invocation,
        }
    }

    /// The shared per-job event sequencer: a cheap clone-able view over the
    /// same state and serialization point the reservations use.
    pub fn sequencer(&self) -> super::sequencer::JobEventSequencer {
        super::sequencer::JobEventSequencer {
            inner: self.inner.clone(),
        }
    }
}

pub struct ChildSession {
    pub(super) permit: EffectPermit,
    pub(super) view: super::ChildBudgetView,
}

impl ChildSession {
    pub fn view(&self) -> &super::ChildBudgetView {
        &self.view
    }
}

impl State {
    pub(super) fn remaining(&self) -> Duration {
        self.limits.deadline.saturating_sub(self.started.elapsed())
    }

    pub(super) fn owner_failure(&mut self) -> EffectAdmissionError {
        self.snapshot.poisoned = true;
        self.cancellation
            .cancel_with_reason(CancellationReason::OwnerFailure);
        EffectOwnerFailure::new(std::io::Error::other("budget owner invariant failed")).into()
    }

    pub(super) fn deadline_expired(&self) -> bool {
        self.remaining().is_zero()
    }

    /// Latches the root deadline reason without overwriting an earlier cause.
    pub(super) fn latch_deadline_expiry(&self) {
        self.cancellation
            .cancel_with_reason(CancellationReason::Budget(
                saya_agent::BudgetDimension::ElapsedTime,
            ));
    }

    pub(super) async fn admit_one(
        &mut self,
        dimension: BudgetDimension,
        amount: u64,
        agent: Option<AgentId>,
        invocation: Option<InvocationId>,
    ) -> Result<EffectPermit, EffectAdmissionError> {
        super::admission::admit_one(self, dimension, amount, agent, invocation).await
    }

    pub(super) async fn finish(
        &mut self,
        permit: EffectPermit,
        settlement: super::settlement::Settlement,
    ) -> Result<(), EffectAdmissionError> {
        super::settlement::finish(self, permit, settlement).await
    }
}
