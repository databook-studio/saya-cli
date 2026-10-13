//! The job-wide budget owner: fresh construction, resume hand-off, and
//! the views over its shared state.

use saya_agent::CancellationToken;
use saya_store::{CodingBudgetPolicyAppend, SqliteStateStore, StoreError};
use saya_types::{AgentId, InvocationId, JobId};
use std::{collections::HashMap, sync::Arc, time::Instant};
use tokio::sync::Mutex;

use super::{BudgetLedgerError, BudgetSnapshot, CountLimits, Counts, State};

#[derive(Clone)]
pub struct JobBudgetLedger {
    pub(in crate::coding::budget) schedule: Arc<tokio::sync::Semaphore>,
    pub(in crate::coding::budget) inner: Arc<Mutex<State>>,
    pub(in crate::coding::budget) cancellation: CancellationToken,
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
        let policy = super::super::fresh::proposed_policy(&limits)?;
        match store.record_coding_budget_policy(&job, &policy).await {
            Ok(
                CodingBudgetPolicyAppend::Recorded | CodingBudgetPolicyAppend::AlreadyPresentSame,
            ) => {}
            Err(StoreError::PolicyConflict) => return Err(BudgetLedgerError::PolicyMismatch),
            Err(_) => return Err(BudgetLedgerError::Store),
        }
        let started = Instant::now();
        let sequence = super::super::fresh::initial_sequence(&store, &job).await?;
        let token_policy = store
            .get_coding_token_policy(&job)
            .await
            .map_err(|_| BudgetLedgerError::Store)?;
        let cancellation = CancellationToken::new();
        Ok(Self {
            schedule: Arc::new(tokio::sync::Semaphore::new(1)),
            inner: Arc::new(Mutex::new(State {
                token_policy,
                quiescent: Arc::new(std::sync::atomic::AtomicBool::new(false)),
                runtime_authority: None,
                runtime_renewal_required: false,
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
                reported_token_ceiling: None,
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
        super::super::resume::resume_ledger(store, job, limits).await
    }

    /// Registers the optional job-wide ceiling on provider-reported usage
    /// as runtime configuration. It is checked before each new model
    /// request, it is not part of the durable budget policy, and a rebuilt
    /// owner does not carry it. One request already in flight can overshoot
    /// the ceiling, and usage the provider did not report is not counted.
    pub async fn set_reported_token_ceiling(&self, ceiling: Option<u64>) {
        self.inner.lock().await.reported_token_ceiling = ceiling;
    }

    pub fn root_view(&self) -> super::super::RootBudgetView {
        super::super::RootBudgetView {
            inner: self.inner.clone(),
        }
    }

    pub(in crate::coding::budget) fn child(
        &self,
        agent: AgentId,
        invocation: InvocationId,
    ) -> super::super::ChildBudgetView {
        super::super::ChildBudgetView {
            inner: self.inner.clone(),
            agent,
            invocation,
        }
    }

    /// The shared per-job event sequencer: a cheap clone-able view over the
    /// same state and serialization point the reservations use.
    pub fn sequencer(&self) -> super::super::sequencer::JobEventSequencer {
        super::super::sequencer::JobEventSequencer {
            inner: self.inner.clone(),
        }
    }
}
