//! Transport observations use the existing live permit and shared event sequencer.
use super::{accounting::owner_error, admission::PermitKind, ledger::State};
use saya_agent::{
    EffectCallKind, EffectOutcome, EffectOwnerFailure, EffectPermit, PhysicalAttempt,
    PhysicalAttemptObservation,
};
use saya_types::{
    AgentId, CodingEventKind, CodingEventV1, InvocationId, TransportCallKind, TransportOutcome,
    TransportSpanData, TransportSpanV1,
};
use std::num::NonZeroU64;

pub(super) async fn observe(
    state: &mut State,
    actor: Option<(&AgentId, &InvocationId)>,
    permit: EffectPermit,
    attempt: PhysicalAttempt,
    observation: PhysicalAttemptObservation,
) -> Result<(), EffectOwnerFailure> {
    if state.snapshot.poisoned {
        return Err(owner_error(state));
    }
    let stored = state
        .permits
        .get(&permit.get())
        .cloned()
        .ok_or_else(|| owner_error(state))?;
    let PermitKind::Physical {
        parent,
        attempt: actual,
    } = stored.kind
    else {
        return Err(owner_error(state));
    };
    if actual != attempt || stored.agent.as_ref().zip(stored.invocation.as_ref()) != actor {
        return Err(owner_error(state));
    }
    let logical = state
        .permits
        .get(&parent)
        .cloned()
        .ok_or_else(|| owner_error(state))?;
    if !matches!(
        logical.kind,
        PermitKind::Model {
            active_attempt: true,
            ..
        }
    ) {
        return Err(owner_error(state));
    }
    let outcome = match observation.outcome() {
        EffectOutcome::NotStarted => TransportOutcome::NotStarted,
        EffectOutcome::Succeeded => TransportOutcome::Completed,
        EffectOutcome::Unknown => TransportOutcome::Unknown,
        _ => return Err(owner_error(state)),
    };
    let call_kind = match attempt.kind() {
        EffectCallKind::Answer => TransportCallKind::Answer,
        EffectCallKind::Continuation => TransportCallKind::Continuation,
        EffectCallKind::Salvage => TransportCallKind::Salvage,
        _ => return Err(owner_error(state)),
    };
    let span = TransportSpanV1::new(TransportSpanData {
        version: 1,
        job_id: state.job.clone(),
        agent_id: stored.agent.clone(),
        invocation_id: stored.invocation.clone(),
        runtime_epoch: state
            .runtime_authority
            .as_ref()
            .and_then(|authority| NonZeroU64::new(authority.epoch)),
        reservation_sequence: stored.reservation,
        parent_reservation_sequence: logical.reservation,
        ordinal: attempt.ordinal(),
        call_kind,
        process_id: observation.process_id(),
        clock_domain: observation.clock_domain().to_owned(),
        dispatch_started_ns: observation.dispatch_started_ns(),
        finished_ns: observation.finished_ns(),
        outcome,
        reported_input_tokens: observation.reported_usage().map(|usage| usage.input_tokens),
        reported_output_tokens: observation
            .reported_usage()
            .map(|usage| usage.output_tokens),
        usage_complete: observation.usage_complete(),
    })
    .map_err(|_| owner_error(state))?;
    let sequence = NonZeroU64::new(state.sequence).ok_or_else(|| owner_error(state))?;
    let event = CodingEventV1::new(
        sequence,
        state.job.clone(),
        stored.agent,
        stored.invocation,
        CodingEventKind::TransportObserved {
            span: Box::new(span),
        },
    )
    .map_err(|_| owner_error(state))?;
    match state.store.record_coding_transport(event).await {
        Ok(saya_store::CodingEventAppend::Appended(actual)) if actual == sequence => {
            state.sequence = state
                .sequence
                .checked_add(1)
                .ok_or_else(|| owner_error(state))?;
            Ok(())
        }
        _ => Err(owner_error(state)),
    }
}
