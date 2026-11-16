//! Sticky, payload-free native owner failure state.

use super::checks::ChildCodingOperations;
use saya_agent::{CancellationReason, CancellationToken};
use saya_harness::coding::child::OwnerFailureSnapshot;
use saya_types::CodingFailureClass;
use std::sync::{Mutex, atomic::Ordering};

#[derive(Debug, Default)]
pub(crate) struct NativeOwnerFailureState(Mutex<Option<OwnerFailureSnapshot>>);

impl NativeOwnerFailureState {
    pub(crate) fn snapshot(&self) -> Option<OwnerFailureSnapshot> {
        *self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Retains the earliest safe primary and at most one later distinct safe
    /// secondary; payloads never enter this shared state.
    pub(crate) fn latch(&self, primary: CodingFailureClass, secondary: Option<CodingFailureClass>) {
        let mut held = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let next = match *held {
            None => OwnerFailureSnapshot::new(primary, secondary),
            Some(existing) => OwnerFailureSnapshot::new(
                existing.primary(),
                existing
                    .secondary()
                    .or((existing.primary() != primary).then_some(primary))
                    .or(secondary),
            ),
        };
        *held = Some(next);
    }
}

impl ChildCodingOperations {
    /// Latches a payload-free fatal owner operation before refusing all later
    /// effects. The supplied invocation token and the ledger receive the same
    /// owner-failure cause so an induced cancellation cannot masquerade as a
    /// user or provider outcome.
    pub(crate) fn fail_owner_operation(
        &self,
        primary: CodingFailureClass,
        secondary: Option<CodingFailureClass>,
        invocation: &CancellationToken,
    ) -> OwnerFailureSnapshot {
        self.owner_failure.latch(primary, secondary);
        self.failed.store(true, Ordering::Release);
        self.ledger
            .cancellation_token()
            .cancel_with_reason(CancellationReason::OwnerFailure);
        invocation.cancel_with_reason(CancellationReason::OwnerFailure);
        self.owner_failure
            .snapshot()
            .expect("fatal owner operation was just latched")
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn first_primary_and_one_later_secondary_are_sticky() {
        let state = NativeOwnerFailureState::default();
        state.latch(CodingFailureClass::ExtensionExchange, None);
        state.latch(CodingFailureClass::MandatoryPersistence, None);
        state.latch(CodingFailureClass::Internal, None);

        assert_eq!(
            state.snapshot(),
            Some(OwnerFailureSnapshot::new(
                CodingFailureClass::ExtensionExchange,
                Some(CodingFailureClass::MandatoryPersistence)
            ))
        );
    }

    #[test]
    fn an_incoming_primary_precedes_its_secondary() {
        let state = NativeOwnerFailureState::default();
        state.latch(CodingFailureClass::ProviderError, None);
        state.latch(
            CodingFailureClass::ExtensionExchange,
            Some(CodingFailureClass::MandatoryPersistence),
        );

        assert_eq!(
            state.snapshot(),
            Some(OwnerFailureSnapshot::new(
                CodingFailureClass::ProviderError,
                Some(CodingFailureClass::ExtensionExchange)
            ))
        );
    }

    #[test]
    fn duplicate_input_keeps_the_secondary_slot_for_the_next_cause() {
        let state = NativeOwnerFailureState::default();
        state.latch(
            CodingFailureClass::ExtensionExchange,
            Some(CodingFailureClass::ExtensionExchange),
        );
        state.latch(CodingFailureClass::MandatoryPersistence, None);

        assert_eq!(
            state.snapshot(),
            Some(OwnerFailureSnapshot::new(
                CodingFailureClass::ExtensionExchange,
                Some(CodingFailureClass::MandatoryPersistence)
            ))
        );
    }
}
