use super::*;

// ---------------------------------------------------------------------------
// Restore eligibility: committed only, and only on a still-matching digest
// ---------------------------------------------------------------------------

#[test]
fn restore_is_only_eligible_for_a_committed_edit_that_still_matches() {
    let edited = committed();
    assert_eq!(
        restore_eligibility(&edited, Some(AFTER_SHA256)),
        RestoreEligibility::Eligible
    );
    assert_eq!(
        restore_eligibility(&edited, Some(BEFORE_SHA256)),
        RestoreEligibility::FileChangedSince
    );
    assert_eq!(
        restore_eligibility(&edited, None),
        RestoreEligibility::FileMissing
    );
    assert_eq!(
        restore_eligibility(&prepared(), Some(AFTER_SHA256)),
        RestoreEligibility::NotCommitted
    );
    assert_eq!(
        restore_eligibility(&prepared(), None),
        RestoreEligibility::NotCommitted
    );
    assert_eq!(
        restore_eligibility(&uncertain(), Some(AFTER_SHA256)),
        RestoreEligibility::EffectUncertain
    );
}

#[test]
fn a_create_edit_must_still_match_its_post_edit_version_to_be_restorable() {
    let created = create_committed();
    assert_eq!(
        restore_eligibility(&created, Some(AFTER_SHA256)),
        RestoreEligibility::Eligible
    );
    assert_eq!(
        restore_eligibility(&created, Some(BEFORE_SHA256)),
        RestoreEligibility::FileChangedSince
    );
}
