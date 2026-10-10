//! Conservative durable fold for a resumed budget owner: rebuild the
//! in-memory owner's admitted counts, unknown counters, retained slot
//! charges, and next event sequence from durable records. Count-only:
//! charged units are never refunded and unreleased slots stay consumed.

use std::{collections::HashMap, sync::Arc, time::Instant};

use saya_agent::CancellationToken;
use saya_store::{CodingBudgetReservationRecord, SqliteStateStore, StoreError};
use saya_types::{
    BudgetDimension, BudgetEffectState, BudgetObservationValue, BudgetReservationV1,
    BudgetSettlementV1, JobId,
};
use tokio::sync::Mutex;

use super::ledger::{
    BudgetLedgerError, BudgetSnapshot, Counts, JobBudgetLedger, State, UsageCoverage,
};

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
    let sequence = super::fresh::resume_sequence(&store, &job).await?;
    let started = Instant::now();
    let cancellation = CancellationToken::new();
    Ok(JobBudgetLedger {
        inner: Arc::new(Mutex::new(State {
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
            snapshot: resumed_snapshot(&folded),
            usage_ever_unknown: folded.usage_ever_unknown,
            usage_ever_incomplete: folded.usage_ever_incomplete,
        })),
        cancellation,
    })
}

async fn fold(store: &SqliteStateStore, job: &JobId) -> Result<Folded, BudgetLedgerError> {
    let mut folded = Folded::default();
    let mut cursor = None;
    loop {
        let page = store
            .list_coding_budget_reservations(job, cursor, 100)
            .await
            .map_err(map_resume_store)?;
        for record in &page.items {
            fold_record(&mut folded, record)?;
            // The store caps a job's total reservations, so the page count
            // cannot wrap.
            folded.reservations += 1;
        }
        let Some(next) = page.next_cursor else { break };
        if Some(next) == cursor {
            return Err(BudgetLedgerError::ResumeRecordCorrupt);
        }
        cursor = Some(next);
    }
    Ok(folded)
}

/// Folds one durable reservation record into the resumed state.
///
/// A record with a settlement is folded unchanged from its charges and
/// observations. A record whose `settlement` is `None` was interrupted
/// mid-effect: its usage and outcome are UNKNOWN, so its reserved
/// charges stay counted (never refunded), the reserved amounts also
/// count toward the matching unknown-usage counters for the count
/// dimensions, the record counts as unsettled, and usage coverage may
/// never read Complete again.
fn fold_record(
    folded: &mut Folded,
    record: &CodingBudgetReservationRecord,
) -> Result<(), BudgetLedgerError> {
    if let Some(settlement) = &record.settlement {
        check_settlement_shape(settlement)?;
    }
    // Every charge adds to the cumulative admitted counts, including
    // NotStarted and Uncertain outcomes: no owner API refunds a charged
    // count. A slot charge is released only by a Settled settlement; a
    // retained charge stays in Counts forever — with no live permits
    // after a restart it is permanently consumed against the cap.
    let released = record
        .settlement
        .as_ref()
        .is_some_and(|settlement| settlement.effect_state() == BudgetEffectState::Settled);
    for charge in record.reservation.charges() {
        let amount = charge.reserved().get();
        match charge.dimension() {
            BudgetDimension::LogicalRequests => add(&mut folded.counts.logical, amount)?,
            BudgetDimension::TransportAttempts => add(&mut folded.counts.transport, amount)?,
            BudgetDimension::ToolCalls => add(&mut folded.counts.tools, amount)?,
            BudgetDimension::ModelRequestSlots if !released => {
                add(&mut folded.counts.model_slots, amount)?
            }
            BudgetDimension::ChildSlots if !released => {
                add(&mut folded.counts.child_slots, amount)?
            }
            BudgetDimension::CommandSlots if !released => {
                add(&mut folded.counts.command_slots, amount)?
            }
            BudgetDimension::ModelRequestSlots
            | BudgetDimension::ChildSlots
            | BudgetDimension::CommandSlots => {}
            _ => return Err(BudgetLedgerError::ResumeRecordCorrupt),
        }
    }
    let Some(settlement) = &record.settlement else {
        folded.usage_ever_unknown = true;
        add(&mut folded.unsettled, 1)?;
        for charge in record.reservation.charges() {
            let amount = charge.reserved().get();
            match charge.dimension() {
                BudgetDimension::LogicalRequests => {
                    add(&mut folded.unknown_logical_requests, amount)?
                }
                BudgetDimension::TransportAttempts => {
                    add(&mut folded.unknown_transport_attempts, amount)?
                }
                BudgetDimension::ToolCalls => add(&mut folded.unknown_tool_calls, amount)?,
                _ => {}
            }
        }
        return Ok(());
    };
    for observation in settlement.observations() {
        let dimension = observation.dimension();
        match observation.value() {
            BudgetObservationValue::Unknown => {
                let amount = charged(&record.reservation, dimension)?;
                match dimension {
                    BudgetDimension::LogicalRequests => {
                        add(&mut folded.unknown_logical_requests, amount)?
                    }
                    BudgetDimension::TransportAttempts => {
                        add(&mut folded.unknown_transport_attempts, amount)?;
                        folded.usage_ever_unknown = true;
                    }
                    BudgetDimension::ToolCalls => add(&mut folded.unknown_tool_calls, amount)?,
                    // D5 residual: an Uncertain slot settlement is unresumable; R04 reconciles.
                    _ => return Err(BudgetLedgerError::ResumeRecordCorrupt),
                }
            }
            BudgetObservationValue::Measured(amount) => {
                if dimension == BudgetDimension::ToolCalls {
                    add(&mut folded.measured_tool_calls, amount)?
                }
            }
            _ => {}
        }
        // Reported usage totals are not durable: a transport observation
        // proves only that the attempt settled, never that its reported
        // usage was complete, so coverage never reads Complete again.
        if dimension == BudgetDimension::TransportAttempts {
            folded.usage_ever_incomplete = true;
        }
    }
    Ok(())
}

/// Refuses a settlement whose observation contradicts its effect state:
/// the owner derives Uncertain exactly from any Unknown observation, so a
/// Settled settlement with an Unknown observation, or an Uncertain one
/// with none, is corrupt. Key mismatches are refused by the store's read
/// path and surface here as corrupt records.
fn check_settlement_shape(settlement: &BudgetSettlementV1) -> Result<(), BudgetLedgerError> {
    let any_unknown = settlement
        .observations()
        .iter()
        .any(|observation| observation.value() == BudgetObservationValue::Unknown);
    if settlement.effect_state() == BudgetEffectState::Settled && any_unknown
        || settlement.effect_state() == BudgetEffectState::Uncertain && !any_unknown
    {
        return Err(BudgetLedgerError::ResumeRecordCorrupt);
    }
    Ok(())
}

/// The charged amount behind one observed dimension; no matching charge is
/// a key mismatch and corrupt.
fn charged(
    reservation: &BudgetReservationV1,
    dimension: BudgetDimension,
) -> Result<u64, BudgetLedgerError> {
    reservation
        .charges()
        .iter()
        .find(|charge| charge.dimension() == dimension)
        .map(|charge| charge.reserved().get())
        .ok_or(BudgetLedgerError::ResumeRecordCorrupt)
}

fn add(total: &mut u64, amount: u64) -> Result<(), BudgetLedgerError> {
    *total = total
        .checked_add(amount)
        .ok_or(BudgetLedgerError::ResumeRecordCorrupt)?;
    Ok(())
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
