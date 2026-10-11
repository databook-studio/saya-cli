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
/// Scope: control events go through the store's public generic control-event
/// append, which accepts only `StateChanged` and `ResultReported` kinds;
/// checkout events now route here under the same ledger mutex, through
/// [`JobEventSequencer::append_checkout_intent`],
/// [`JobEventSequencer::append_checkout_operation_intent`], and
/// [`JobEventSequencer::append_checkout_outcome`] into the store's
/// `record_checkout_create_intent`, `record_checkout_operation_intent`, and
/// `record_checkout_operation_outcome`. Producers of other event kinds (approvals) still need their own
/// owner-side wiring, and adopters needing multi-event transactions (the
/// R13b approval consume) need their own owner step, because one call here
/// consumes exactly one sequence.
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

/// Which store append one routed sequencer call drains through.
enum Route {
    /// The store's generic control-event append.
    Control,
    /// The store's checkout create-intent append.
    CheckoutIntent,
    /// The store's checkout operation-intent append.
    CheckoutOperationIntent,
    /// The store's checkout operation-outcome append.
    CheckoutOutcome,
}

impl JobEventSequencer {
    /// Allocates the next exact sequence, hands it to `build` to fill into
    /// the event, appends through the store's exact-next-sequence API, and
    /// advances the shared counter only after the store accepts.
    pub async fn append_event(
        &self,
        build: impl FnOnce(u64) -> CodingEventV1,
    ) -> Result<AppendedEvent, SequencerError> {
        self.append_routed(Route::Control, build).await
    }

    /// The checkout owner step's create-intent append: the owner-side wiring
    /// this struct's docs previously said was missing. The closure must fill
    /// a `CheckoutCreateIntent` event at the allocated sequence; the store
    /// accepts it only through `record_checkout_create_intent`.
    pub async fn append_checkout_intent(
        &self,
        build: impl FnOnce(u64) -> CodingEventV1,
    ) -> Result<AppendedEvent, SequencerError> {
        self.append_routed(Route::CheckoutIntent, build).await
    }

    /// The checkout owner step's operation-intent append, mirroring
    /// [`JobEventSequencer::append_checkout_intent`] for a
    /// `CheckoutOperationIntent` event through the store's
    /// `record_checkout_operation_intent`.
    pub async fn append_checkout_operation_intent(
        &self,
        build: impl FnOnce(u64) -> CodingEventV1,
    ) -> Result<AppendedEvent, SequencerError> {
        self.append_routed(Route::CheckoutOperationIntent, build)
            .await
    }

    /// The checkout owner step's operation-outcome append, mirroring
    /// [`JobEventSequencer::append_checkout_intent`] for a
    /// `CheckoutOperationOutcome` event through the store's
    /// `record_checkout_operation_outcome`.
    pub async fn append_checkout_outcome(
        &self,
        build: impl FnOnce(u64) -> CodingEventV1,
    ) -> Result<AppendedEvent, SequencerError> {
        self.append_routed(Route::CheckoutOutcome, build).await
    }

    /// The single append point behind every route: one exact sequence
    /// allocation under the ledger mutex, the same pre-checks, and the same
    /// store-result handling, so control and checkout events drain one
    /// contiguous counter.
    async fn append_routed(
        &self,
        route: Route,
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
        let append = match route {
            Route::Control => state.store.append_coding_event(event).await,
            Route::CheckoutIntent => state.store.record_checkout_create_intent(event).await,
            Route::CheckoutOperationIntent => {
                state.store.record_checkout_operation_intent(event).await
            }
            Route::CheckoutOutcome => state.store.record_checkout_operation_outcome(event).await,
        };
        match append {
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
            Ok(_) => {
                state.owner_failure();
                Err(SequencerError::Conflict(allocated.get()))
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
