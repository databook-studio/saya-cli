//! Claim lifecycle rules: which origins may confirm directly, which states
//! are recallable, and the guarantee that contract errors never leak input.

use super::*;

#[test]
fn test_10_origin_may_confirm_directly() {
    use ClaimOrigin::*;
    assert!(UserExplicit.may_confirm_directly());
    // ADR 0002 §4: a reviewed team file enters confirmed within its declared
    // scope, so TeamFile is confirmable without a per-claim review step.
    assert!(TeamFile.may_confirm_directly());
    assert!(!AssistantInferred.may_confirm_directly());
}

#[test]
fn test_10_status_is_recallable() {
    use ClaimStatus::*;
    assert!(Confirmed.is_recallable());
    assert!(!Candidate.is_recallable());
    assert!(!Rejected.is_recallable());
    assert!(!Stale.is_recallable());
    assert!(!Forgotten.is_recallable());
}

#[test]
fn test_11_errors_never_leak_input() {
    let input = "SUPERSECRETVALUE";
    for error in &[
        ContractError::InvalidProfileIdentity,
        ContractError::InvalidClaimId,
        ContractError::EmptyName,
        ContractError::NameTooLong,
        ContractError::ControlCharacter,
        ContractError::EmptyText,
        ContractError::TextTooLong,
        ContractError::TooManyColumns,
        ContractError::ColumnCountMismatch,
        ContractError::EmptyColumns,
    ] {
        let msg = error.to_string();
        assert!(!msg.contains(input), "error {error:?} leaks input");
    }
}
