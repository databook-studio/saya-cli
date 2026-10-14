use std::num::NonZeroU64;

use saya_types::{CodingEventError, CodingEventKind, CodingEventV1, JobId, Sha256Digest};

fn reconciled(reservation: u64) -> CodingEventKind {
    CodingEventKind::BudgetReconciled {
        reservation_sequence: NonZeroU64::new(reservation).unwrap(),
        note_sha256: Sha256Digest::parse(&"e".repeat(64)).unwrap(),
    }
}

fn append_at(sequence: u64, kind: CodingEventKind) -> Result<CodingEventV1, CodingEventError> {
    CodingEventV1::new(
        NonZeroU64::new(sequence).unwrap(),
        JobId::parse("job-1").unwrap(),
        None,
        None,
        kind,
    )
}

#[test]
fn a_reconciliation_must_come_after_the_reservation_it_names() {
    assert!(append_at(14, reconciled(13)).is_ok(), "positive control");
    for same_or_later in [14, 15] {
        assert_eq!(
            append_at(14, reconciled(same_or_later)).unwrap_err(),
            CodingEventError::InvalidEventLink,
            "reservation {same_or_later} is not before sequence 14"
        );
    }
}

#[test]
fn a_reconciliation_with_a_malformed_note_digest_is_refused_on_decode() {
    let good = super::event_disposition_wire::BUDGET_RECONCILED;
    assert!(
        CodingEventV1::from_json_bytes(good.as_bytes()).is_ok(),
        "positive control"
    );
    let bad = good.replace(&"e".repeat(64), "not-a-digest");
    assert_ne!(good, bad, "the fixture digest was replaced");
    assert_eq!(
        CodingEventV1::from_json_bytes(bad.as_bytes()).unwrap_err(),
        CodingEventError::InvalidDigest
    );
}
