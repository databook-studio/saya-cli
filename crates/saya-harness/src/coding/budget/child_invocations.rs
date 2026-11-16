use super::{AppendedEvent, JobEventSequencer, SequencerError};
use saya_store::{ChildInvocationError, CodingEventAppend};
use saya_types::{AgentId, ChildInvocationRecord, CodingEventKind, CodingEventV1, InvocationId};
use std::num::NonZeroU64;

#[derive(Debug, thiserror::Error)]
pub enum ChildContinuationError {
    #[error("child continuation precondition is no longer valid")]
    Refused,
    #[error(transparent)]
    Owner(#[from] SequencerError),
}
impl JobEventSequencer {
    pub async fn append_child_invocation(
        &self,
        agent: &AgentId,
        invocation: &InvocationId,
        record: ChildInvocationRecord,
    ) -> Result<AppendedEvent, ChildContinuationError> {
        let mut state = self.inner.lock().await;
        if state.snapshot.poisoned {
            return Err(SequencerError::Poisoned.into());
        }
        if state.cancellation.is_cancelled() || state.runtime_renewal_required {
            return Err(ChildContinuationError::Refused);
        }
        let authority = state
            .runtime_authority
            .clone()
            .ok_or(ChildContinuationError::Refused)?;
        let sequence = NonZeroU64::new(state.sequence).ok_or(SequencerError::Poisoned)?;
        let event = CodingEventV1::new(
            sequence,
            state.job.clone(),
            Some(agent.clone()),
            Some(invocation.clone()),
            CodingEventKind::ChildInvocation {
                record: Box::new(record),
            },
        )
        .map_err(|_| SequencerError::SequenceMismatch)?;
        match state
            .store
            .record_coding_child_invocation(event, &authority)
            .await
        {
            Ok(CodingEventAppend::Appended(actual)) if actual == sequence => {
                state.sequence = state.sequence.checked_add(1).ok_or_else(|| {
                    state.owner_failure();
                    SequencerError::Poisoned
                })?;
                Ok(AppendedEvent { sequence })
            }
            Err(ChildInvocationError::Refused) => Err(ChildContinuationError::Refused),
            Err(ChildInvocationError::Store(error)) => {
                state.owner_failure();
                Err(SequencerError::Store(sequence.get(), error).into())
            }
            Ok(_) => {
                state.owner_failure();
                Err(SequencerError::Conflict(sequence.get()).into())
            }
        }
    }
}
