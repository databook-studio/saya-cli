//! The durable fold: paging reservation records and folding each into the
//! resumed counters.

use saya_store::{CodingBudgetReservationRecord, SqliteStateStore};
use saya_types::{
    BudgetDimension, BudgetEffectState, BudgetObservationValue, BudgetReservationV1,
    BudgetSettlementV1, JobId,
};

use super::super::ledger::BudgetLedgerError;
use super::{Folded, map_resume_store};

pub(super) async fn fold(
    store: &SqliteStateStore,
    job: &JobId,
) -> Result<Folded, BudgetLedgerError> {
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
/// charges stay counted (never refunded) and the reserved amounts also
/// count toward the matching unknown-usage counters for the count
/// dimensions. A record charging a usage dimension counts as unsettled
/// and usage coverage may never read Complete again; a slot-only record
/// (child or command slots) retains its charges as permanently consumed
/// capacity but carries no usage, so it is neither.
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
            BudgetDimension::InputTokens | BudgetDimension::OutputTokens => {
                let charged = record
                    .settlement
                    .as_ref()
                    .and_then(|settlement| {
                        settlement
                            .observations()
                            .iter()
                            .find(|observation| observation.dimension() == charge.dimension())
                    })
                    .and_then(|observation| match observation.value() {
                        BudgetObservationValue::Measured(value) => Some(value),
                        _ => None,
                    })
                    .unwrap_or(amount);
                add(&mut folded.counts.tokens, charged)?;
            }
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
        if carries_usage(&record.reservation) {
            folded.usage_ever_unknown = true;
            add(&mut folded.unsettled, 1)?;
        }
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
                    BudgetDimension::InputTokens | BudgetDimension::OutputTokens => {
                        folded.usage_ever_unknown = true;
                    }
                    // An Uncertain slot settlement is unresumable until it is
                    // reconciled; a reconciled one keeps its slot charge
                    // consumed (never released: it is not `Settled`), exactly
                    // as an open reservation does.
                    BudgetDimension::ModelRequestSlots
                    | BudgetDimension::ChildSlots
                    | BudgetDimension::CommandSlots
                        if record.reconciled => {}
                    _ => return Err(BudgetLedgerError::ResumeRecordCorrupt),
                }
            }
            BudgetObservationValue::Measured(amount) if dimension == BudgetDimension::ToolCalls => {
                add(&mut folded.measured_tool_calls, amount)?
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

/// Whether the reservation charges a usage dimension: the count
/// dimensions and reported tokens. Slot dimensions retain their charges
/// as consumed capacity without describing usage.
fn carries_usage(reservation: &BudgetReservationV1) -> bool {
    reservation.charges().iter().any(|charge| {
        matches!(
            charge.dimension(),
            BudgetDimension::LogicalRequests
                | BudgetDimension::TransportAttempts
                | BudgetDimension::ToolCalls
                | BudgetDimension::InputTokens
                | BudgetDimension::OutputTokens
        )
    })
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
