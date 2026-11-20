//! What a readiness view says about the walk behind its source generation: the
//! one refusal that is reported (a checkout that kept changing under the walk)
//! and every refusal that keeps the handling it had.

use saya_harness::coding::integration::{CandidateError, DeltaError, EvidenceError};
use saya_types::{ManagedCheckoutId, Sha256Digest};

use super::{CodingError, reported, vanished_class};
use crate::commands::coding::prepare::SourceGeneration;

/// The refusal the readiness walk produces when it gives up.
fn walked(error: DeltaError) -> CodingError {
    CodingError::owner(EvidenceError::Unverifiable(CandidateError::Walk(error)))
}

#[test]
fn a_walk_that_gave_up_on_a_vanished_entry_is_reported_and_stays_unknown() {
    let error = walked(DeltaError::Vanished);
    assert_eq!(
        vanished_class(&error),
        Some("vanished"),
        "the class behind the warning names what happened, and nothing else"
    );
    assert!(
        reported(Err(error)).is_none(),
        "the readiness view stays unknown, as it did when every refusal was swallowed"
    );
}

#[test]
fn every_other_refusal_keeps_todays_handling() {
    for class in [
        DeltaError::Io,
        DeltaError::Cancelled,
        DeltaError::DeadlineExceeded,
        DeltaError::TooManyFiles,
        DeltaError::UnsafePath,
    ] {
        let error = walked(class);
        assert_eq!(
            vanished_class(&error),
            None,
            "only a vanished entry is reported: {class:?}"
        );
        assert!(
            reported(Err(error)).is_none(),
            "a refusal that is not a vanished entry still yields no generation: {class:?}"
        );
    }
    assert!(
        reported(Err(CodingError::InvalidRequest)).is_none(),
        "an error with no walk behind it yields no generation either"
    );
}

#[test]
fn a_generation_that_the_walk_produced_is_reported_unchanged() {
    let generation = SourceGeneration {
        digest: Sha256Digest::parse(&"a".repeat(64)).expect("a sixty-four character digest"),
        candidate: ManagedCheckoutId::parse("candidate-a").expect("a candidate id"),
    };
    let reported = reported(Ok(generation)).expect("a produced generation is kept");
    assert_eq!(reported.candidate.as_str(), "candidate-a");
}
