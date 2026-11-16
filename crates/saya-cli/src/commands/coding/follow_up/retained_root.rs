//! Preserve historical unknown usage while proving the narrow prepared-root case.
use super::super::{CodingError, NativeCodingJob};
use saya_harness::coding::budget::{BudgetSnapshot, has_unknown_effect};
use saya_types::{
    BudgetDimension, BudgetEffectState, BudgetObservationValue, ChildInvocationRecord,
    CodingEventKind,
};
use std::{
    collections::BTreeSet,
    time::{Duration, Instant},
};

pub(in crate::commands::coding) async fn allows(
    job: &NativeCodingJob,
    budget: &BudgetSnapshot,
) -> Result<bool, CodingError> {
    let Some(resumed) = job
        .application
        .store
        .latest_coding_run_event(job.job_id())
        .await
        .map_err(CodingError::owner)?
    else {
        return Ok(false);
    };
    let CodingEventKind::RuntimeResumed {
        revocation_epoch,
        policy_sha256,
    } = resumed.event()
    else {
        return Ok(false);
    };
    if revocation_epoch.get() != job.epoch || policy_sha256 != &job.policy_digest {
        return Ok(false);
    }
    let boundary = resumed.sequence();
    let started = Instant::now();
    let (mut cursor, mut scanned) = (None, 0usize);
    let (mut unknown, mut unsettled) = (0u64, 0u64);
    let mut causal = BTreeSet::new();
    loop {
        if job.cancellation.is_cancelled() || started.elapsed() > Duration::from_secs(5) {
            return Ok(false);
        }
        let page = job
            .application
            .store
            .list_coding_budget_reservations(job.job_id(), cursor, 100)
            .await
            .map_err(CodingError::owner)?;
        for record in page.items {
            scanned += 1;
            // Matches the authoritative store cap; work stays bounded if it changes.
            if scanned > 10_000 {
                return Ok(false);
            }
            let reservation = &record.reservation;
            let sequence = reservation.reservation_sequence();
            // Current admissions are not part of the immutable resume counters.
            if sequence > boundary {
                continue;
            }
            // Unknown usage of a model request is not an unknown effect: only
            // an open reservation or an unknown effect observation needs the
            // narrow root-tool proof below.
            let is_unknown = record
                .settlement
                .as_ref()
                .is_none_or(|settlement| has_unknown_effect(settlement.observations()));
            if !is_unknown {
                continue;
            }
            let charges = reservation.charges();
            if !record.reconciled
                || reservation.agent_id().is_some()
                || reservation.invocation_id().is_some()
                || charges.len() != 1
                || charges[0].dimension() != BudgetDimension::ToolCalls
                || charges[0].reserved().get() != 1
            {
                return Ok(false);
            }
            if let Some(settlement) = &record.settlement {
                if settlement.effect_state() != BudgetEffectState::Uncertain
                    || settlement.observations().len() != 1
                    || settlement.observations()[0].value() != BudgetObservationValue::Unknown
                {
                    return Ok(false);
                }
            } else {
                unsettled += 1;
            }
            unknown += 1;
            causal.insert(sequence);
        }
        let Some(next) = page.next_cursor else {
            break;
        };
        if Some(next) == cursor {
            return Ok(false);
        }
        cursor = Some(next);
    }
    if unknown != budget.unknown_tool_calls || unsettled != budget.unsettled_reservations {
        return Ok(false);
    }
    // These are job-global usage facts, never grants for another child's scope.
    // Require actual same-job typed adjacency and reconciliation before this
    // exact fresh runtime; the target's independent scope/CAS checks still apply.
    // The projection flag alone is insufficient: require the actual typed fact
    // before this exact fresh runtime, without rewriting or refunding anything.
    let mut prepared = causal.clone();
    let mut reconciled = causal.clone();
    let mut cursor = causal.first().copied();
    let mut scanned = 0usize;
    while !prepared.is_empty() || !reconciled.is_empty() {
        if job.cancellation.is_cancelled() || started.elapsed() > Duration::from_secs(5) {
            return Ok(false);
        }
        let page = job
            .application
            .store
            .list_coding_events(job.job_id(), cursor, 100)
            .await
            .map_err(CodingError::owner)?;
        for event in page.events {
            scanned += 1;
            if scanned > 65_536 || event.sequence() >= boundary {
                return Ok(false);
            }
            if event.job_id() != job.job_id() {
                return Ok(false);
            }
            match event.event() {
                CodingEventKind::ChildInvocation { record } => {
                    if matches!(
                        record.as_ref(),
                        ChildInvocationRecord::FollowUpPrepared { .. }
                    ) && let Some(previous) = event
                        .sequence()
                        .get()
                        .checked_sub(1)
                        .and_then(std::num::NonZeroU64::new)
                    {
                        prepared.remove(&previous);
                    }
                }
                CodingEventKind::BudgetReconciled {
                    reservation_sequence,
                    ..
                } => {
                    reconciled.remove(reservation_sequence);
                }
                _ => {}
            }
            if prepared.is_empty() && reconciled.is_empty() {
                return Ok(true);
            }
        }
        let Some(next) = page.next_cursor else {
            return Ok(false);
        };
        if Some(next) == cursor {
            return Ok(false);
        }
        cursor = Some(next);
    }
    Ok(true)
}
