//! The evidence kind a `contract_propose` call attaches, decided from the
//! turn's observations and recall receipt.
//!
//! Pure and store-free so the mapping is unit-testable without persistence
//! (the store has no public read for `contract_evidence`, so the kind is
//! asserted here, at the decision, rather than round-tripped). See the SPEC
//! REVIEW for 3c: the spec's `ExplicitUserStatement`-when-touched mapping is a
//! defect — that kind means "the user said this", which contradicts the
//! `AssistantInferred` origin every proposal carries. The touched/untouched
//! distinction maps onto `SuccessfulReadQuery` vs `RepeatedObservation` instead.
//!
//! A proposal about an object whose claims were supplied this turn must not earn
//! `TOUCHED` (`SuccessfulReadQuery`) purely because a query touched it: the query
//! was caused by the supplied claim, not independent confirmation. It receives
//! `UNTOUCHED` (`RepeatedObservation`).

use saya_store::EvidenceKind;

/// The kind for a proposal about an object the turn's observations did touch
/// (a succeeded query referenced it): the assistant saw the object in a read
/// that worked, which is what `SuccessfulReadQuery` names.
pub(super) const TOUCHED: EvidenceKind = EvidenceKind::SuccessfulReadQuery;

/// The kind for a proposal about an object the turn never touched, or whose
/// claims were supplied to the model this turn: a weaker, secondhand signal the
/// assistant inferred without independent query confirmation.
pub(super) const UNTOUCHED: EvidenceKind = EvidenceKind::RepeatedObservation;

/// Decides the evidence kind for a proposal.
///
/// Returns [`TOUCHED`] (`SuccessfulReadQuery`) when a query touched the proposed
/// object this turn AND no claims for this object were supplied to the model
/// this turn.
///
/// Returns [`UNTOUCHED`] (`RepeatedObservation`) if the object was untouched, OR
/// if claims for this object were supplied this turn (preventing a claim from
/// becoming its own evidence).
pub(super) fn decide_evidence_kind(touched: bool, supplied: bool) -> EvidenceKind {
    if touched && !supplied {
        TOUCHED
    } else {
        UNTOUCHED
    }
}

/// Checks whether `proposed` matches any supplied object name exactly on the
/// qualified name and case-insensitively, following the contracts layer
/// conventions.
pub(super) fn is_supplied_object(supplied_objects: &[String], proposed: &str) -> bool {
    supplied_objects
        .iter()
        .any(|supplied| supplied.eq_ignore_ascii_case(proposed))
}

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

    #[test]
    fn decide_evidence_kind_touched_and_not_supplied() {
        assert_eq!(decide_evidence_kind(true, false), TOUCHED);
    }

    #[test]
    fn decide_evidence_kind_touched_and_supplied_gets_untouched() {
        // Claim supplied this turn must not become its own evidence.
        assert_eq!(decide_evidence_kind(true, true), UNTOUCHED);
    }

    #[test]
    fn decide_evidence_kind_untouched_and_not_supplied() {
        assert_eq!(decide_evidence_kind(false, false), UNTOUCHED);
    }

    #[test]
    fn decide_evidence_kind_untouched_and_supplied() {
        assert_eq!(decide_evidence_kind(false, true), UNTOUCHED);
    }

    #[test]
    fn is_supplied_object_exact_and_case_insensitive() {
        let supplied = vec![
            "catalog.public.orders".to_string(),
            "DB.SCHEMA.USERS".to_string(),
        ];
        assert!(is_supplied_object(&supplied, "catalog.public.orders"));
        assert!(is_supplied_object(&supplied, "CATALOG.PUBLIC.ORDERS"));
        assert!(is_supplied_object(&supplied, "Catalog.Public.Orders"));
        assert!(is_supplied_object(&supplied, "db.schema.users"));
        assert!(is_supplied_object(&supplied, "DB.SCHEMA.USERS"));

        // Non-matches:
        assert!(!is_supplied_object(&supplied, "catalog.public.other"));
        assert!(!is_supplied_object(&supplied, "public.orders"));
        assert!(!is_supplied_object(&supplied, "orders"));
    }
}
