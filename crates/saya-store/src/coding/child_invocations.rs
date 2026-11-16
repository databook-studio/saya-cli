//! Atomic, fenced current-invocation rotation; generic terminal transitions stay closed.
mod guards;
use super::{CodingEventAppend, CodingRuntimeAuthority, events::append_event_in_tx};
use crate::{SqliteStateStore, StoreError};
use saya_types::{AgentId, ChildInvocationRecord, CodingEventKind, CodingEventV1, JobId};
use sqlx::{Row, Sqlite, Transaction};

#[derive(Debug, thiserror::Error)]
pub enum ChildInvocationError {
    #[error("child continuation precondition changed or is not eligible")]
    Refused,
    #[error(transparent)]
    Store(#[from] StoreError),
}

impl SqliteStateStore {
    pub async fn coding_child_invocation_history(
        &self,
        job: &JobId,
        agent: &AgentId,
    ) -> Result<Vec<CodingEventV1>, StoreError> {
        let mut tx = self
            .pool()
            .await?
            .begin()
            .await
            .map_err(|_| StoreError::Unavailable)?;
        let history = history(&mut tx, job, agent).await?;
        tx.commit().await.map_err(|_| StoreError::Unavailable)?;
        Ok(history)
    }

    pub async fn record_coding_child_invocation(
        &self,
        event: CodingEventV1,
        authority: &CodingRuntimeAuthority,
    ) -> Result<CodingEventAppend, ChildInvocationError> {
        let (Some(agent), Some(invocation)) = (event.agent_id(), event.invocation_id()) else {
            return Err(StoreError::Invalid.into());
        };
        let CodingEventKind::ChildInvocation { record } = event.event() else {
            return Err(StoreError::Invalid.into());
        };
        let mut tx = self
            .pool()
            .await?
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(|_| StoreError::Unavailable)?;
        match super::runtime::check_fence(&mut tx, event.job_id(), Some(authority)).await {
            Err(StoreError::Conflict) => return Err(ChildInvocationError::Refused),
            other => other?,
        }
        let row=sqlx::query("SELECT c.current_invocation_id,c.state,j.state AS job_state FROM coding_children c JOIN coding_jobs j ON j.job_id=c.job_id WHERE c.job_id=? AND c.agent_id=?")
            .bind(event.job_id().as_str()).bind(agent.as_str()).fetch_optional(&mut *tx).await.map_err(|_|StoreError::Unavailable)?.ok_or(ChildInvocationError::Refused)?;
        let current: String = row
            .try_get("current_invocation_id")
            .map_err(|_| StoreError::Invalid)?;
        let state: String = row.try_get("state").map_err(|_| StoreError::Invalid)?;
        if row
            .try_get::<String, _>("job_state")
            .map_err(|_| StoreError::Invalid)?
            != "running"
        {
            return Err(ChildInvocationError::Refused);
        }
        let history = history(&mut tx, event.job_id(), agent).await?;
        match record.as_ref() {
            ChildInvocationRecord::ScopeBound { checkout_id, .. } => {
                if current != invocation.as_str() || state != "queued" || !history.is_empty() {
                    return Err(ChildInvocationError::Refused);
                }
                guards::checkout(&mut tx, &event, checkout_id).await?;
                guards::no_reservations(&mut tx, &event, invocation).await?;
            }
            ChildInvocationRecord::FollowUpPrepared {
                checkout_id,
                previous_invocation_id,
                previous_completion_sequence,
                scope_sha256,
                follow_up_index,
                ..
            } => {
                if current != previous_invocation_id.as_str() || state != "completed" {
                    return Err(ChildInvocationError::Refused);
                }
                guards::checkout(&mut tx, &event, checkout_id).await?;
                guards::binding(&history, checkout_id, scope_sha256)?;
                let count=history.iter().filter(|event|matches!(event.event(),CodingEventKind::ChildInvocation{record} if matches!(record.as_ref(),ChildInvocationRecord::FollowUpPrepared{..}))).count();
                if count + 1 != usize::from(*follow_up_index) || *follow_up_index > 3 {
                    return Err(ChildInvocationError::Refused);
                }
                guards::approvals(&mut tx, &event).await?;
                guards::completion(
                    &mut tx,
                    &event,
                    previous_invocation_id,
                    *previous_completion_sequence,
                )
                .await?;
                guards::settled(&mut tx, &event, previous_invocation_id).await?;
                guards::no_reservations(&mut tx, &event, invocation).await?;
                if history
                    .iter()
                    .any(|event| event.invocation_id() == Some(invocation))
                {
                    return Err(ChildInvocationError::Refused);
                }
            }
            ChildInvocationRecord::FollowUpAdopted {
                prepared_sequence,
                source_sha256,
                instruction_sha256,
            } => {
                if current != invocation.as_str() || state != "queued" {
                    return Err(ChildInvocationError::Refused);
                }
                let prepared = history
                    .iter()
                    .find(|event| {
                        event.sequence() == *prepared_sequence
                            && event.invocation_id() == Some(invocation)
                    })
                    .ok_or(ChildInvocationError::Refused)?;
                let CodingEventKind::ChildInvocation { record } = prepared.event() else {
                    return Err(ChildInvocationError::Refused);
                };
                let ChildInvocationRecord::FollowUpPrepared {
                    checkout_id,
                    scope_sha256,
                    source_sha256: source,
                    instruction_sha256: instruction,
                    ..
                } = record.as_ref()
                else {
                    return Err(ChildInvocationError::Refused);
                };
                if source != source_sha256 || instruction != instruction_sha256 {
                    return Err(ChildInvocationError::Refused);
                }
                guards::binding(&history, checkout_id, scope_sha256)?;
                guards::checkout(&mut tx, &event, checkout_id).await?;
                guards::no_reservations(&mut tx, &event, invocation).await?;
                guards::approvals(&mut tx, &event).await?;
                guards::not_adopted(&mut tx, &event, *prepared_sequence, authority.epoch).await?;
            }
            _ => return Err(StoreError::Invalid.into()),
        }
        let append = append_event_in_tx(&mut tx, event.clone()).await?;
        if let ChildInvocationRecord::FollowUpPrepared { .. } = record.as_ref() {
            sqlx::query("UPDATE coding_children SET current_invocation_id=?,state='queued',result_source_sha256=NULL,result_outcome=NULL,updated_unix_ms=? WHERE job_id=? AND agent_id=?")
                .bind(invocation.as_str()).bind(super::records::now()?).bind(event.job_id().as_str()).bind(agent.as_str())
                .execute(&mut *tx).await.map_err(|_|StoreError::Unavailable)?;
        }
        tx.commit().await.map_err(|_| StoreError::Unavailable)?;
        Ok(append)
    }
}

async fn history(
    tx: &mut Transaction<'_, Sqlite>,
    job: &JobId,
    agent: &AgentId,
) -> Result<Vec<CodingEventV1>, StoreError> {
    let rows=sqlx::query("SELECT sequence,event_json FROM coding_events WHERE job_id=? AND json_extract(event_json,'$.agent_id')=? AND json_extract(event_json,'$.event.type')='child_invocation' AND json_extract(event_json,'$.event.record.operation') IN ('scope_bound','follow_up_prepared') ORDER BY sequence LIMIT 5")
        .bind(job.as_str()).bind(agent.as_str()).fetch_all(&mut **tx).await.map_err(|_|StoreError::Unavailable)?;
    if rows.len() > 4 {
        return Err(StoreError::Invalid);
    }
    rows.into_iter()
        .map(|row| {
            let text: String = row.try_get("event_json").map_err(|_| StoreError::Invalid)?;
            let event = CodingEventV1::from_json_bytes(text.as_bytes())
                .map_err(super::events::event_error)?;
            let sequence: i64 = row.try_get("sequence").map_err(|_| StoreError::Invalid)?;
            if event.job_id() != job
                || event.agent_id() != Some(agent)
                || i64::try_from(event.sequence().get()).ok() != Some(sequence)
            {
                return Err(StoreError::Invalid);
            }
            Ok(event)
        })
        .collect()
}
