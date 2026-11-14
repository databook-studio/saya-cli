use super::{AppendedEvent, JobEventSequencer, SequencerError};
use saya_store::CodingEventAppend;
use saya_types::{CodingEventKind, CodingEventV1, JoinedResultV1};
use std::num::NonZeroU64;

impl JobEventSequencer {
    /// The scheduler calls this after joining the real child. It shares the
    /// reservation sequencer and atomically records both state and output.
    pub async fn append_completion(
        &self,
        result: &JoinedResultV1,
    ) -> Result<AppendedEvent, SequencerError> {
        let mut state = self.inner.lock().await;
        if result.job_id() != &state.job {
            return Err(SequencerError::ForeignJob);
        }
        if state.snapshot.poisoned {
            return Err(SequencerError::Poisoned);
        }
        let Some(sequence) = NonZeroU64::new(state.sequence) else {
            state.owner_failure();
            return Err(SequencerError::Poisoned);
        };
        let event = CodingEventV1::new(
            sequence,
            result.job_id().clone(),
            Some(result.agent_id().clone()),
            Some(result.invocation_id().clone()),
            CodingEventKind::StateChanged {
                state: result.terminal_state(),
            },
        )
        .map_err(|_| SequencerError::SequenceMismatch)?;
        match state.store.record_coding_completion(event, result).await {
            Ok(CodingEventAppend::Appended(actual)) if actual == sequence => {
                let Some(next) = state.sequence.checked_add(1) else {
                    state.owner_failure();
                    return Err(SequencerError::Poisoned);
                };
                state.sequence = next;
                Ok(AppendedEvent { sequence })
            }
            Ok(_) => {
                state.owner_failure();
                Err(SequencerError::Conflict(sequence.get()))
            }
            Err(error) => {
                state.owner_failure();
                Err(SequencerError::Store(sequence.get(), error))
            }
        }
    }
}
