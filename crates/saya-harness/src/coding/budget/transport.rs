//! Transport observations use the existing live permit and shared event sequencer.
use super::{accounting::owner_error, admission::PermitKind, ledger::State};
use saya_agent::{
    EffectCallKind, EffectOutcome, EffectOwnerFailure, EffectPermit, PhysicalAttempt,
    PhysicalAttemptObservation, PhysicalRequestCorrelation,
};
use saya_types::{
    AgentId, CodingEventKind, CodingEventV1, InvocationId, Sha256Digest, TransportCallKind,
    TransportOutcome, TransportSpanData, TransportSpanV1,
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
    let correlation = observation
        .correlation()
        .map(|correlation| persisted_correlation(state, correlation))
        .transpose()?;
    let correlation = correlation.unwrap_or_default();
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
        reported_cached_input_tokens: observation
            .reported_usage()
            .and_then(|usage| usage.cached_input_tokens),
        reported_reasoning_tokens: observation
            .reported_usage()
            .and_then(|usage| usage.reasoning_tokens),
        reported_cost_micros: observation
            .reported_usage()
            .and_then(|usage| usage.cost_micros),
        usage_complete: observation.usage_complete(),
        conversation_correlation_id: correlation.conversation_correlation_id,
        invocation_correlation_id: correlation.invocation_correlation_id,
        actor_correlation_sha256: correlation.actor_correlation_sha256,
        request_ordinal: correlation.request_ordinal,
        projection_revision: correlation.projection_revision,
        projection_size_bytes: correlation.projection_size_bytes,
        cache_key_digest: correlation.cache_key_digest,
        thinking: observation.thinking(),
        streamed: observation.streamed(),
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

#[derive(Default)]
struct PersistedCorrelation {
    conversation_correlation_id: Option<NonZeroU64>,
    invocation_correlation_id: Option<NonZeroU64>,
    actor_correlation_sha256: Option<Sha256Digest>,
    request_ordinal: Option<NonZeroU64>,
    projection_revision: Option<NonZeroU64>,
    projection_size_bytes: Option<NonZeroU64>,
    cache_key_digest: Option<u64>,
}

fn persisted_correlation(
    state: &mut State,
    correlation: &PhysicalRequestCorrelation,
) -> Result<PersistedCorrelation, EffectOwnerFailure> {
    let conversation =
        NonZeroU64::new(correlation.conversation_id().get()).ok_or_else(|| owner_error(state))?;
    let invocation =
        NonZeroU64::new(correlation.invocation_id().get()).ok_or_else(|| owner_error(state))?;
    let request_ordinal =
        NonZeroU64::new(correlation.request_ordinal()).ok_or_else(|| owner_error(state))?;
    let projection_revision =
        NonZeroU64::new(correlation.projection_revision()).ok_or_else(|| owner_error(state))?;
    let actor = correlation
        .actor_digest()
        .map(|digest| {
            Sha256Digest::parse(&digest_hex(digest.as_bytes())).map_err(|_| owner_error(state))
        })
        .transpose()?;
    let projection_size =
        u64::try_from(correlation.projection_size_bytes()).map_err(|_| owner_error(state))?;
    Ok(PersistedCorrelation {
        conversation_correlation_id: Some(conversation),
        invocation_correlation_id: Some(invocation),
        actor_correlation_sha256: actor,
        request_ordinal: Some(request_ordinal),
        projection_revision: Some(projection_revision),
        projection_size_bytes: Some(
            NonZeroU64::new(projection_size).ok_or_else(|| owner_error(state))?,
        ),
        cache_key_digest: correlation.cache_key_digest(),
    })
}

fn digest_hex(digest: &[u8; 32]) -> String {
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}
