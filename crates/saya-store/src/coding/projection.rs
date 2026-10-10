use saya_types::{CodingEventKind, CodingEventV1, CodingResultOutcome, JobState};
use sqlx::{Row, Sqlite, Transaction};

use super::records::{now, state};
use crate::StoreError;

pub(crate) async fn apply_common_projection(
    transaction: &mut Transaction<'_, Sqlite>,
    event: &CodingEventV1,
) -> Result<(), StoreError> {
    match event.event() {
        CodingEventKind::StateChanged { state: next } => {
            let job_state = job_state(transaction, event).await?;
            if terminal(job_state) {
                return Err(StoreError::Conflict);
            }
            match (event.agent_id(), event.invocation_id()) {
                (None, None) => update_job(transaction, event, job_state, *next).await,
                (Some(agent_id), Some(invocation_id)) => {
                    update_child(
                        transaction,
                        event,
                        agent_id.as_str(),
                        invocation_id.as_str(),
                        *next,
                    )
                    .await
                }
                _ => Err(StoreError::Invalid),
            }
        }
        CodingEventKind::ResultReported {
            source_sha256,
            outcome,
        } => {
            if terminal(job_state(transaction, event).await?) {
                return Err(StoreError::Conflict);
            }
            let (Some(agent_id), Some(invocation_id)) = (event.agent_id(), event.invocation_id())
            else {
                return Err(StoreError::Invalid);
            };
            let row = sqlx::query("SELECT current_invocation_id, state, result_source_sha256 FROM coding_children WHERE job_id=? AND agent_id=?")
                .bind(event.job_id().as_str())
                .bind(agent_id.as_str())
                .fetch_optional(&mut **transaction)
                .await
                .map_err(|_| StoreError::Unavailable)?
                .ok_or(StoreError::NotFound)?;
            let current: String = row
                .try_get("current_invocation_id")
                .map_err(|_| StoreError::Invalid)?;
            let child_state = state(
                &row.try_get::<String, _>("state")
                    .map_err(|_| StoreError::Invalid)?,
            )?;
            let reported: Option<String> = row
                .try_get("result_source_sha256")
                .map_err(|_| StoreError::Invalid)?;
            if current != invocation_id.as_str() || terminal(child_state) || reported.is_some() {
                return Err(StoreError::Conflict);
            }
            sqlx::query("UPDATE coding_children SET result_source_sha256=?, result_outcome=?, updated_unix_ms=? WHERE job_id=? AND agent_id=?")
                .bind(source_sha256.as_str())
                .bind(outcome_name(*outcome)?)
                .bind(now()?)
                .bind(event.job_id().as_str())
                .bind(agent_id.as_str())
                .execute(&mut **transaction)
                .await
                .map_err(|_| StoreError::Unavailable)?;
            Ok(())
        }
        _ => Ok(()),
    }
}

async fn job_state(
    transaction: &mut Transaction<'_, Sqlite>,
    event: &CodingEventV1,
) -> Result<JobState, StoreError> {
    let value: Option<String> = sqlx::query_scalar("SELECT state FROM coding_jobs WHERE job_id=?")
        .bind(event.job_id().as_str())
        .fetch_optional(&mut **transaction)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    value
        .map(|value| state(&value))
        .transpose()?
        .ok_or(StoreError::NotFound)
}

async fn update_job(
    transaction: &mut Transaction<'_, Sqlite>,
    event: &CodingEventV1,
    current: JobState,
    next: JobState,
) -> Result<(), StoreError> {
    transition(current, next)?;
    sqlx::query("UPDATE coding_jobs SET state=?, updated_unix_ms=? WHERE job_id=?")
        .bind(state_name(next)?)
        .bind(now()?)
        .bind(event.job_id().as_str())
        .execute(&mut **transaction)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    Ok(())
}

async fn update_child(
    transaction: &mut Transaction<'_, Sqlite>,
    event: &CodingEventV1,
    agent_id: &str,
    invocation_id: &str,
    next: JobState,
) -> Result<(), StoreError> {
    let row = sqlx::query(
        "SELECT current_invocation_id, state FROM coding_children WHERE job_id=? AND agent_id=?",
    )
    .bind(event.job_id().as_str())
    .bind(agent_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|_| StoreError::Unavailable)?
    .ok_or(StoreError::NotFound)?;
    let current_invocation: String = row
        .try_get("current_invocation_id")
        .map_err(|_| StoreError::Invalid)?;
    if current_invocation != invocation_id {
        return Err(StoreError::Conflict);
    }
    let current = state(
        &row.try_get::<String, _>("state")
            .map_err(|_| StoreError::Invalid)?,
    )?;
    if terminal(current) {
        return Err(StoreError::Conflict);
    }
    transition(current, next)?;
    sqlx::query(
        "UPDATE coding_children SET state=?, updated_unix_ms=? WHERE job_id=? AND agent_id=?",
    )
    .bind(state_name(next)?)
    .bind(now()?)
    .bind(event.job_id().as_str())
    .bind(agent_id)
    .execute(&mut **transaction)
    .await
    .map_err(|_| StoreError::Unavailable)?;
    Ok(())
}

fn transition(current: JobState, next: JobState) -> Result<(), StoreError> {
    if current == JobState::WaitingForApproval && next == JobState::Queued {
        return Err(StoreError::Conflict);
    }
    current
        .transition_to(next)
        .map(|_| ())
        .map_err(|_| StoreError::Conflict)
}

fn terminal(state: JobState) -> bool {
    matches!(
        state,
        JobState::Completed | JobState::Failed | JobState::Cancelled
    )
}

fn state_name(state: JobState) -> Result<&'static str, StoreError> {
    Ok(match state {
        JobState::Queued => "queued",
        JobState::Running => "running",
        JobState::WaitingForApproval => "waiting_for_approval",
        JobState::Paused => "paused",
        JobState::Completed => "completed",
        JobState::Failed => "failed",
        JobState::Cancelled => "cancelled",
        JobState::Uncertain => "uncertain",
        _ => return Err(StoreError::Invalid),
    })
}

fn outcome_name(outcome: CodingResultOutcome) -> Result<&'static str, StoreError> {
    Ok(match outcome {
        CodingResultOutcome::Completed => "completed",
        CodingResultOutcome::Failed => "failed",
        CodingResultOutcome::Cancelled => "cancelled",
        CodingResultOutcome::Uncertain => "uncertain",
        _ => return Err(StoreError::Invalid),
    })
}
