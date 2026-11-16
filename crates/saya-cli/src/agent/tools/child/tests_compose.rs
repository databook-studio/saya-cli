//! Composition boundary tests for the child view's trusted bindings.

use saya_types::{JobId, ManagedCheckoutId, SnapshotId};

use super::super::tests::{checkout, implementer_view};
use super::validate_view_ownership;

#[test]
fn authority_view_must_match_verified_job_checkout_and_snapshot() {
    let checkout_id = checkout("owned");
    let view = implementer_view(&checkout_id);
    let job_id = JobId::parse("job").unwrap();
    let snapshot_id = SnapshotId::parse("snapshot").unwrap();

    assert!(validate_view_ownership(&view, &checkout_id, &job_id, &snapshot_id).is_ok());
    assert!(
        validate_view_ownership(
            &view,
            &ManagedCheckoutId::parse("other").unwrap(),
            &job_id,
            &snapshot_id,
        )
        .is_err()
    );
    assert!(
        validate_view_ownership(
            &view,
            &checkout_id,
            &JobId::parse("other-job").unwrap(),
            &snapshot_id,
        )
        .is_err()
    );
    assert!(
        validate_view_ownership(
            &view,
            &checkout_id,
            &job_id,
            &SnapshotId::parse("other-snapshot").unwrap(),
        )
        .is_err()
    );
}
