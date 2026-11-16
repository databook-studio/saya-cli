//! Fingerprint versioning is what keeps stale claims out, so a digest must
//! report the format it was built under, not the current one.

use super::*;

#[test]
fn fingerprint_reports_the_version_it_was_built_under() {
    let table = valid_table();
    let live = SchemaFingerprint::of_table(DatabaseObjectKind::Table, &table);
    assert_eq!(live.version(), FINGERPRINT_VERSION);
    assert!(live.is_current_format());

    // A digest loaded from storage under an older format keeps that version.
    let stored = SchemaFingerprint::from_parts(FINGERPRINT_VERSION - 1, live.as_str())
        .expect("valid digest");
    assert_eq!(stored.version(), FINGERPRINT_VERSION - 1);
    assert!(!stored.is_current_format());
    assert_ne!(stored, live);
}

#[test]
fn fingerprint_from_parts_rejects_a_malformed_digest() {
    for bad in ["", "xyz", &"A".repeat(64), &"a".repeat(63), &"a".repeat(65)] {
        assert_eq!(
            SchemaFingerprint::from_parts(FINGERPRINT_VERSION, bad),
            Err(ContractError::InvalidFingerprint),
            "digest {bad:?} should be rejected",
        );
    }
}

#[test]
fn fingerprint_round_trips_through_its_stored_parts() {
    let live = SchemaFingerprint::of_table(DatabaseObjectKind::View, &valid_table());
    let restored =
        SchemaFingerprint::from_parts(live.version(), live.as_str()).expect("valid digest");
    assert_eq!(restored, live);
}
