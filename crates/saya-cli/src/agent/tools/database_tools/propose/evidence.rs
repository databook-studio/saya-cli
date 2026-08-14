//! The evidence kind a `contract_propose` call attaches, decided from the
//! turn's observations.
//!
//! Pure and store-free so the mapping is unit-testable without persistence
//! (the store has no public read for `contract_evidence`, so the kind is
//! asserted here, at the decision, rather than round-tripped). See the SPEC
//! REVIEW for 3c: the spec's `ExplicitUserStatement`-when-touched mapping is a
//! defect — that kind means "the user said this", which contradicts the
//! `AssistantInferred` origin every proposal carries. The touched/untouched
//! distinction maps onto `SuccessfulReadQuery` vs `RepeatedObservation` instead.

use saya_store::EvidenceKind;

/// The kind for a proposal about an object the turn's observations did touch
/// (a succeeded query referenced it): the assistant saw the object in a read
/// that worked, which is what `SuccessfulReadQuery` names.
pub(super) const TOUCHED: EvidenceKind = EvidenceKind::SuccessfulReadQuery;

/// The kind for a proposal about an object the turn never touched: a weaker,
/// secondhand signal the assistant inferred without querying the object.
pub(super) const UNTOUCHED: EvidenceKind = EvidenceKind::RepeatedObservation;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn touched_is_a_successful_read_not_an_explicit_user_statement() {
        // ExplicitUserStatement would contradict the AssistantInferred origin;
        // SuccessfulReadQuery is the honest name for "a read query saw this".
        assert_eq!(TOUCHED, EvidenceKind::SuccessfulReadQuery);
        assert_ne!(TOUCHED, EvidenceKind::ExplicitUserStatement);
    }

    #[test]
    fn untouched_is_repeated_observation() {
        assert_eq!(UNTOUCHED, EvidenceKind::RepeatedObservation);
    }

    #[test]
    fn the_two_kinds_differ() {
        // The touched/untouched distinction must be observable in the kind.
        assert_ne!(TOUCHED, UNTOUCHED);
    }
}
