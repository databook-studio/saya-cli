use super::ledger::State;
use saya_agent::{EffectAdmissionError, EffectOwnerFailure};
use saya_store::CodingEventAppend;
use saya_types::{
    BudgetCharge, BudgetDimension, BudgetReservationV1, CodingEventKind, CodingEventV1,
};
use std::num::NonZeroU64;

pub(super) async fn append_reservation(
    state: &mut State,
    sequence: NonZeroU64,
    charges: Vec<(BudgetDimension, u64)>,
    agent: Option<saya_types::AgentId>,
    invocation: Option<saya_types::InvocationId>,
) -> Result<(), EffectAdmissionError> {
    let charges = charges
        .into_iter()
        .map(|(dimension, amount)| {
            let amount = NonZeroU64::new(amount).ok_or_else(|| corrupt(state))?;
            BudgetCharge::new(dimension, None, None, amount).map_err(|_| corrupt(state))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let reservation = BudgetReservationV1::new(
        state.job.clone(),
        sequence,
        agent.clone(),
        invocation.clone(),
        charges,
    )
    .map_err(|_| corrupt(state))?;
    let event = CodingEventV1::new(
        sequence,
        state.job.clone(),
        agent,
        invocation,
        CodingEventKind::BudgetReserved {
            reservation: Box::new(reservation),
        },
    )
    .map_err(|_| corrupt(state))?;
    match state
        .store
        .reserve_coding_budget_fenced(event, state.runtime_authority.as_ref())
        .await
    {
        Ok(CodingEventAppend::Appended(actual)) if actual == sequence => Ok(()),
        _ => Err(corrupt(state)),
    }
}

pub(super) async fn settle_reservation(
    state: &mut State,
    reservation_sequence: NonZeroU64,
    effect_state: saya_types::BudgetEffectState,
    observations: Vec<saya_types::BudgetObservation>,
) -> Result<(), EffectAdmissionError> {
    let settlement_sequence = NonZeroU64::new(state.sequence).ok_or_else(|| corrupt(state))?;
    let settlement = saya_types::BudgetSettlementV1::new(
        state.job.clone(),
        reservation_sequence,
        settlement_sequence,
        effect_state,
        observations,
    )
    .map_err(|_| corrupt(state))?;
    let event = CodingEventV1::new(
        settlement_sequence,
        state.job.clone(),
        None,
        None,
        CodingEventKind::BudgetSettled {
            settlement: Box::new(settlement),
        },
    )
    .map_err(|_| corrupt(state))?;
    match state.store.settle_coding_budget(event).await {
        Ok(CodingEventAppend::Appended(actual)) if actual == settlement_sequence => {
            state.sequence = state
                .sequence
                .checked_add(1)
                .ok_or_else(|| corrupt(state))?;
            Ok(())
        }
        _ => Err(corrupt(state)),
    }
}

impl State {
    /// The owner's reservation entry point: bounded, cancel-aware, and
    /// serialized through the durable append. It lives beside the other
    /// reservation-append paths it serializes through.
    pub(super) async fn reserve(
        &mut self,
        charges: Vec<(BudgetDimension, u64)>,
        agent: Option<saya_types::AgentId>,
        invocation: Option<saya_types::InvocationId>,
    ) -> Result<NonZeroU64, EffectAdmissionError> {
        if self.quiescent.load(std::sync::atomic::Ordering::Acquire)
            || (self.runtime_renewal_required && self.runtime_authority.is_some())
        {
            return Err(EffectAdmissionError::Denied);
        }
        if self.snapshot.poisoned || self.reservations >= super::ledger::MAX_RESERVATIONS {
            return Err(EffectAdmissionError::Denied);
        }
        if self.cancellation.is_cancelled() {
            return Err(EffectAdmissionError::Denied);
        }
        if self.deadline_expired() {
            self.latch_deadline_expiry();
            return Err(EffectAdmissionError::Denied);
        }
        if self.permits.len() >= super::ledger::MAX_LIVE_PERMITS
            || self
                .reservations
                .checked_add(1)
                .is_none_or(|next| next > super::ledger::MAX_RESERVATIONS)
        {
            return Err(EffectAdmissionError::Denied);
        }
        let sequence = NonZeroU64::new(self.sequence).ok_or_else(|| self.owner_failure())?;
        append_reservation(self, sequence, charges, agent, invocation).await?;
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or_else(|| self.owner_failure())?;
        self.reservations += 1;
        Ok(sequence)
    }
}

fn corrupt(state: &mut State) -> EffectAdmissionError {
    state.snapshot.poisoned = true;
    state
        .cancellation
        .cancel_with_reason(saya_agent::CancellationReason::OwnerFailure);
    EffectOwnerFailure::new(std::io::Error::other("durable budget event failed")).into()
}
