use super::ledger::State;
use saya_store::{CodingEventAppend, StoreError};
use saya_types::CodingEventV1;
use std::{num::NonZeroU64, sync::Arc};
use thiserror::Error;
use tokio::sync::Mutex;

/// The shared per-job control-event append point.
///
/// Every clone borrows the same ledger state and mutex that budget
/// reservations serialize through, so reservations and sequencer appends
/// drain one exact contiguous counter. The store accepts an event only at
/// `last + 1` (`append_event_in_tx` in `saya-store/src/coding/events.rs`).
/// On `AlreadyPresent`, a sequence conflict, or any other store failure the
/// owner is poisoned through the same reservation-append failure path
/// (`State::owner_failure`) and the error is returned typed: the sequencer
/// never resequences, retries with a different number, or reads
/// `MAX(sequence)`. Poisoned owners refuse further appends; ordinary
/// cancellation latches do not block, because a cancelled job still needs
/// its final control events.
///
/// Scope: appends go through the store's public generic control-event
/// append, which accepts only `StateChanged` and `ResultReported` kinds —
/// producers of other event kinds (approvals, checkouts) still need their
/// own owner-side wiring, and adopters needing multi-event transactions
/// (the R13b approval consume) need their own owner step, because one call
/// here consumes exactly one sequence.
#[derive(Clone)]
pub struct JobEventSequencer {
    pub(super) inner: Arc<Mutex<State>>,
}

/// The exact sequence the store durably accepted for the appended event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppendedEvent {
    pub sequence: NonZeroU64,
}

/// Why one sequencer append failed. A refusal detectable before any append
/// (a foreign job identity, or a closure returning an event whose sequence
/// differs from the allocated one) is a caller error and leaves the owner
/// usable, the same class as an admission denial: nothing was consumed, no
/// store call was attempted, so the ledger's invariants are intact.
/// Everything observed at or after the append attempt poisons the owner,
/// matching the reservation-append failure convention.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum SequencerError {
    /// Caller bug refused before any append; the owner stays usable.
    #[error("the event names a job the ledger does not own")]
    ForeignJob,
    /// Caller bug refused before any append; the owner stays usable.
    #[error("the closure filled a sequence other than the allocated one")]
    SequenceMismatch,
    #[error("the owner is poisoned; no further appends are accepted")]
    Poisoned,
    #[error("the identical event at sequence {0} is already durable")]
    AlreadyPresent(u64),
    #[error("the store refused an append at sequence {0}")]
    Conflict(u64),
    #[error("the store failed to append at sequence {0}: {1}")]
    Store(u64, StoreError),
}

impl JobEventSequencer {
    /// Allocates the next exact sequence, hands it to `build` to fill into
    /// the event, appends through the store's exact-next-sequence API, and
    /// advances the shared counter only after the store accepts.
    pub async fn append_event(
        &self,
        build: impl FnOnce(u64) -> CodingEventV1,
    ) -> Result<AppendedEvent, SequencerError> {
        let mut state = self.inner.lock().await;
        if state.snapshot.poisoned {
            return Err(SequencerError::Poisoned);
        }
        let Some(allocated) = NonZeroU64::new(state.sequence) else {
            state.owner_failure();
            return Err(SequencerError::Poisoned);
        };
        let event = build(allocated.get());
        if event.job_id() != &state.job {
            return Err(SequencerError::ForeignJob);
        }
        if event.sequence() != allocated {
            return Err(SequencerError::SequenceMismatch);
        }
        match state.store.append_coding_event(event).await {
            Ok(CodingEventAppend::Appended(actual)) if actual == allocated => {
                let Some(next) = state.sequence.checked_add(1) else {
                    state.owner_failure();
                    return Err(SequencerError::Poisoned);
                };
                state.sequence = next;
                Ok(AppendedEvent {
                    sequence: allocated,
                })
            }
            Ok(CodingEventAppend::Appended(_)) => {
                state.owner_failure();
                Err(SequencerError::Conflict(allocated.get()))
            }
            Ok(CodingEventAppend::AlreadyPresent(actual)) => {
                state.owner_failure();
                Err(SequencerError::AlreadyPresent(actual.get()))
            }
            Err(StoreError::Conflict) => {
                state.owner_failure();
                Err(SequencerError::Conflict(allocated.get()))
            }
            Err(error) => {
                state.owner_failure();
                Err(SequencerError::Store(allocated.get(), error))
            }
        }
    }
}
