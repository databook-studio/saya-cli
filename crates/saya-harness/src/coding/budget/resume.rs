//! Conservative durable fold for a resumed budget owner: rebuild the
//! in-memory owner's admitted counts, unknown counters, retained slot
//! charges, and next event sequence from durable records. Count-only:
//! charged units are never refunded and unreleased slots stay consumed.

use std::{collections::HashMap, sync::Arc, time::Instant};

use saya_agent::CancellationToken;
use saya_store::{SqliteStateStore, StoreError};
use saya_types::JobId;
use tokio::sync::Mutex;

use super::ledger::{
    BudgetLedgerError, BudgetSnapshot, Counts, JobBudgetLedger, State, UsageCoverage,
};

mod fold;

use fold::fold;

/// The fold's durable output: exactly the parts of [`State`] that are
/// rebuilt from history, never from the new process's configuration.
#[derive(Default)]
pub(super) struct Folded {
    pub(super) counts: Counts,
    /// Total reservation records folded; counts against the lifetime cap.
    pub(super) reservations: u64,
    pub(super) unknown_logical_requests: u64,
    pub(super) unknown_transport_attempts: u64,
    pub(super) unknown_tool_calls: u64,
    pub(super) measured_tool_calls: u64,
    /// Reservation records folded with no settlement: interrupted
    /// effects whose usage is unknown.
    pub(super) unsettled: u64,
    pub(super) usage_ever_unknown: bool,
    pub(super) usage_ever_incomplete: bool,
}

/// Rebuilds a resumed ledger from durable history. The proposed `limits`
/// only authenticate the recorded policy; the resumed ceilings come from
/// the stored policy, never from this process's configuration. The
/// deadline length is policy-authenticated, but elapsed time is NOT
/// durably recorded: the resumed owner restarts its deadline clock and
/// does not claim elapsed continuity.
pub(super) async fn resume_ledger(
    store: SqliteStateStore,
    job: JobId,
    limits: super::ledger::CountLimits,
) -> Result<JobBudgetLedger, BudgetLedgerError> {
    let token_policy = store
        .get_coding_token_policy(&job)
        .await
        .map_err(map_resume_store)?;
    let runtime_authority = store
        .coding_recovery_status(&job)
        .await
        .map_err(map_resume_store)?
        .authority;
    let proposed = super::fresh::proposed_policy(&limits)?;
    let stored = store
        .get_coding_budget_policy(&job)
        .await
        .map_err(|_| BudgetLedgerError::Store)?
        .ok_or(BudgetLedgerError::PolicyMissing)?;
    if stored.policy_sha256 != proposed.digest() {
        return Err(BudgetLedgerError::PolicyMismatch);
    }
    let policy = stored.policy;
    let durable = super::ledger::CountLimits {
        logical_requests: policy.logical_requests(),
        transport_attempts: policy.transport_attempts(),
        tool_calls: policy.tool_calls(),
        model_slots: policy.model_slots(),
        child_slots: policy.child_slots(),
        deadline: policy.deadline(),
    };
    let folded = fold(&store, &job).await?;
    if token_policy
        .as_ref()
        .is_some_and(|policy| folded.counts.tokens > policy.data().total_tokens)
    {
        return Err(BudgetLedgerError::ResumeRecordCorrupt);
    }
    let sequence = super::fresh::resume_sequence(&store, &job).await?;
    let started = Instant::now();
    let cancellation = CancellationToken::new();
    let snapshot = resumed_snapshot(&folded);
    Ok(JobBudgetLedger {
        schedule: Arc::new(tokio::sync::Semaphore::new(1)),
        inner: Arc::new(Mutex::new(State {
            token_policy,
            quiescent: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            runtime_authority,
            runtime_renewal_required: true,
            store,
            job,
            limits: durable,
            started,
            sequence,
            next_id: 1,
            reservations: folded.reservations,
            counts: folded.counts,
            permits: HashMap::new(),
            groups: HashMap::new(),
            local: HashMap::new(),
            cancellation: cancellation.clone(),
            snapshot,
            usage_ever_unknown: folded.usage_ever_unknown,
            usage_ever_incomplete: folded.usage_ever_incomplete,
            reported_token_ceiling: None,
        })),
        cancellation,
    })
}

fn map_resume_store(error: StoreError) -> BudgetLedgerError {
    if matches!(error, StoreError::Invalid) {
        BudgetLedgerError::ResumeRecordCorrupt
    } else {
        BudgetLedgerError::Store
    }
}

fn resumed_snapshot(folded: &Folded) -> BudgetSnapshot {
    let coverage = if folded.usage_ever_unknown {
        UsageCoverage::Unknown
    } else if folded.usage_ever_incomplete {
        UsageCoverage::Partial
    } else {
        UsageCoverage::Unavailable
    };
    BudgetSnapshot {
        logical_requests: folded.counts.logical,
        transport_attempts: folded.counts.transport,
        tool_calls: folded.counts.tools,
        model_slots_active: folded.counts.model_slots,
        child_slots_active: folded.counts.child_slots,
        command_slots_active: folded.counts.command_slots,
        unknown_logical_requests: folded.unknown_logical_requests,
        unknown_transport_attempts: folded.unknown_transport_attempts,
        unknown_tool_calls: folded.unknown_tool_calls,
        measured_tool_calls: folded.measured_tool_calls,
        unsettled_reservations: folded.unsettled,
        usage_coverage: coverage,
        ..BudgetSnapshot::default()
    }
}
