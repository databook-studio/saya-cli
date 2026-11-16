use super::ChildInvocationError;
use crate::StoreError;
use saya_types::{
    ChildInvocationRecord, CodingEventKind, CodingEventV1, CodingResultOutcome, InvocationId,
    JobId, ManagedCheckoutId, ManagedCheckoutLifecycle, RepositoryId, Sha256Digest,
};
use sqlx::{Sqlite, Transaction};
use std::num::NonZeroU64;

pub(super) fn binding(
    history: &[CodingEventV1],
    checkout: &ManagedCheckoutId,
    scope: &Sha256Digest,
) -> Result<(), ChildInvocationError> {
    if history.iter().filter(|event|matches!(event.event(),CodingEventKind::ChildInvocation{record} if matches!(record.as_ref(),ChildInvocationRecord::ScopeBound{checkout_id,scope_sha256} if checkout_id==checkout && scope_sha256==scope))).count()!=1 { return Err(ChildInvocationError::Refused); }
    Ok(())
}
pub(super) async fn checkout(
    tx: &mut Transaction<'_, Sqlite>,
    event: &CodingEventV1,
    id: &ManagedCheckoutId,
) -> Result<(), ChildInvocationError> {
    let Some(record) =
        super::super::checkout_reads::managed_checkout_in_tx(tx, event.job_id(), id).await?
    else {
        // An in-place job has no managed checkout row: its one synthetic
        // checkout id is derived from the job's own folder id, so the root and
        // every helper name the same folder. Accept exactly that id and
        // nothing else.
        return in_place_checkout(tx, event.job_id(), id).await;
    };
    // A child's scope binds the job's shared working checkout W, which is
    // agent-less: helpers work in W directly, so an agent-less checkout of
    // this job is a valid target. A checkout that names an owning agent must
    // still be that exact child (per-child records).
    if record.current_lifecycle != ManagedCheckoutLifecycle::Active
        || record
            .ownership
            .agent_id()
            .is_some_and(|owner| event.agent_id() != Some(owner))
    {
        return Err(ChildInvocationError::Refused);
    }
    let pending:i64=sqlx::query_scalar("SELECT COUNT(*) FROM coding_checkout_operations o LEFT JOIN coding_events e ON e.job_id=o.job_id AND e.sequence=o.outcome_sequence WHERE o.job_id=? AND o.checkout_id=? AND COALESCE(json_extract(e.event_json,'$.event.operation.state'),'') NOT IN ('completed','failed')")
        .bind(event.job_id().as_str()).bind(id.as_str()).fetch_one(&mut **tx).await.map_err(|_|StoreError::Unavailable)?;
    if pending != 0 {
        return Err(ChildInvocationError::Refused);
    }
    Ok(())
}
pub(super) async fn completion(
    tx: &mut Transaction<'_, Sqlite>,
    event: &CodingEventV1,
    previous: &InvocationId,
    sequence: NonZeroU64,
) -> Result<(), ChildInvocationError> {
    let row=sqlx::query("SELECT c.acknowledged,c.sequence,c.agent_id,c.invocation_id,c.result_bytes,e.event_json FROM coding_completions c JOIN coding_events e ON e.job_id=c.job_id AND e.sequence=c.sequence WHERE c.job_id=? AND c.sequence=? AND c.agent_id=? AND c.invocation_id=?")
        .bind(event.job_id().as_str()).bind(i64::try_from(sequence.get()).map_err(|_|StoreError::Invalid)?).bind(event.agent_id().ok_or(StoreError::Invalid)?.as_str()).bind(previous.as_str())
        .fetch_optional(&mut **tx).await.map_err(|_|StoreError::Unavailable)?.ok_or(ChildInvocationError::Refused)?;
    let completion = super::super::result_reads::decode(&row, event.job_id())?;
    let result = completion.result;
    if result.outcome() != CodingResultOutcome::Completed {
        return Err(ChildInvocationError::Refused);
    }
    Ok(())
}
pub(super) async fn no_reservations(
    tx: &mut Transaction<'_, Sqlite>,
    event: &CodingEventV1,
    invocation: &InvocationId,
) -> Result<(), ChildInvocationError> {
    let count:i64=sqlx::query_scalar("SELECT COUNT(*) FROM coding_events WHERE job_id=? AND json_extract(event_json,'$.agent_id')=? AND json_extract(event_json,'$.invocation_id')=? AND json_extract(event_json,'$.event.type')='budget_reserved'")
        .bind(event.job_id().as_str()).bind(event.agent_id().ok_or(StoreError::Invalid)?.as_str()).bind(invocation.as_str()).fetch_one(&mut **tx).await.map_err(|_|StoreError::Unavailable)?;
    if count != 0 {
        return Err(ChildInvocationError::Refused);
    }
    Ok(())
}
/// Refuses a continuation while an earlier invocation has an open or unknown
/// effect. Unknown model-attempt usage alone does not.
pub(super) async fn settled(
    tx: &mut Transaction<'_, Sqlite>,
    event: &CodingEventV1,
    invocation: &InvocationId,
) -> Result<(), ChildInvocationError> {
    let unsettled = format!(
        "SELECT COUNT(*) FROM coding_budget_reservations r JOIN coding_events b ON b.job_id=r.job_id AND b.sequence=r.reservation_sequence LEFT JOIN coding_events s ON s.job_id=r.job_id AND s.sequence=r.settled_by_sequence WHERE r.job_id=? AND json_extract(b.event_json,'$.agent_id')=? AND json_extract(b.event_json,'$.invocation_id')=? AND r.reconciled_by_sequence IS NULL AND ({} OR {})",
        super::super::unresolved::unresolved("s.event_json"),
        super::super::unresolved::effect_unknown("s.event_json")
    );
    let count: i64 = sqlx::query_scalar(&unsettled)
        .bind(event.job_id().as_str())
        .bind(event.agent_id().ok_or(StoreError::Invalid)?.as_str())
        .bind(invocation.as_str())
        .fetch_one(&mut **tx)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    if count != 0 {
        return Err(ChildInvocationError::Refused);
    }
    Ok(())
}
pub(super) async fn not_adopted(
    tx: &mut Transaction<'_, Sqlite>,
    event: &CodingEventV1,
    prepared: NonZeroU64,
    epoch: u64,
) -> Result<(), ChildInvocationError> {
    let fresh:i64=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM coding_events WHERE job_id=? AND sequence>? AND json_extract(event_json,'$.event.type')='runtime_resumed' AND json_extract(event_json,'$.event.revocation_epoch')=?)")
        .bind(event.job_id().as_str()).bind(i64::try_from(prepared.get()).map_err(|_|StoreError::Invalid)?).bind(i64::try_from(epoch).map_err(|_|StoreError::Invalid)?).fetch_one(&mut **tx).await.map_err(|_|StoreError::Unavailable)?;
    if fresh != 1 {
        return Err(ChildInvocationError::Refused);
    }
    let count:i64=sqlx::query_scalar("SELECT COUNT(*) FROM coding_events WHERE job_id=? AND json_extract(event_json,'$.agent_id')=? AND json_extract(event_json,'$.invocation_id')=? AND json_extract(event_json,'$.event.type')='child_invocation' AND json_extract(event_json,'$.event.record.operation')='follow_up_adopted' AND sequence>COALESCE((SELECT MAX(sequence) FROM coding_events WHERE job_id=? AND json_extract(event_json,'$.event.type')='runtime_resumed' AND json_extract(event_json,'$.event.revocation_epoch')=?),0)")
        .bind(event.job_id().as_str()).bind(event.agent_id().ok_or(StoreError::Invalid)?.as_str()).bind(event.invocation_id().ok_or(StoreError::Invalid)?.as_str()).bind(event.job_id().as_str()).bind(i64::try_from(epoch).map_err(|_|StoreError::Invalid)?).fetch_one(&mut **tx).await.map_err(|_|StoreError::Unavailable)?;
    if prepared >= event.sequence() || count != 0 {
        return Err(ChildInvocationError::Refused);
    }
    Ok(())
}

/// Whether `id` is exactly the synthetic checkout id derived from the job's
/// own in-place folder id. A managed job's repository id is never `loc-…`, so
/// this cannot admit a managed checkout's id.
async fn in_place_checkout(
    tx: &mut Transaction<'_, Sqlite>,
    job: &JobId,
    id: &ManagedCheckoutId,
) -> Result<(), ChildInvocationError> {
    let repository: Option<String> =
        sqlx::query_scalar("SELECT repository_id FROM coding_jobs WHERE job_id=?")
            .bind(job.as_str())
            .fetch_optional(&mut **tx)
            .await
            .map_err(|_| StoreError::Unavailable)?;
    let matches = repository
        .as_deref()
        .and_then(|value| RepositoryId::parse(value).ok())
        .and_then(|repository| repository.in_place_checkout_id())
        .is_some_and(|expected| expected == *id);
    if matches {
        Ok(())
    } else {
        Err(ChildInvocationError::Refused)
    }
}

pub(super) async fn approvals(
    tx: &mut Transaction<'_, Sqlite>,
    event: &CodingEventV1,
) -> Result<(), ChildInvocationError> {
    let count:i64=sqlx::query_scalar("SELECT COUNT(*) FROM coding_approval_requests WHERE job_id=? AND closed_by_sequence IS NULL")
        .bind(event.job_id().as_str()).fetch_one(&mut **tx).await.map_err(|_|StoreError::Unavailable)?;
    if count != 0 {
        return Err(ChildInvocationError::Refused);
    }
    Ok(())
}
