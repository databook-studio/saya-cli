use super::*;

// ---------------------------------------------------------------------------
// The state/after-image identity rules
// ---------------------------------------------------------------------------

#[test]
fn a_committed_record_requires_an_after_image() {
    let mut record = committed();
    record.after = None;
    assert_eq!(
        record.validate(),
        Err(CheckpointError::CommittedRequiresAfter)
    );
    assert_eq!(
        EditCheckpoint::from_json_bytes(&bytes_of(&record)),
        Err(CheckpointError::CommittedRequiresAfter)
    );
}

#[test]
fn a_prepared_or_uncertain_record_must_not_carry_an_after_image() {
    for state in [CheckpointState::Prepared, CheckpointState::Uncertain] {
        let mut record = record(
            state.clone(),
            BeforeImage::Present(version(BEFORE_SHA256, 16)),
        );
        record.after = Some(version(AFTER_SHA256, 16));
        assert_eq!(record.validate(), Err(CheckpointError::StateDisallowsAfter));
        assert_eq!(
            EditCheckpoint::from_json_bytes(&bytes_of(&record)),
            Err(CheckpointError::StateDisallowsAfter)
        );
    }
}

// ---------------------------------------------------------------------------
// Digest identity: 64 lowercase hex, drift detection only
// ---------------------------------------------------------------------------

#[test]
fn a_bad_digest_shape_is_refused() {
    let mut record = committed();
    for bad in ["0123", &"z".repeat(64), &BEFORE_SHA256.to_ascii_uppercase()] {
        record.after = Some(version(bad, 16));
        assert_eq!(record.validate(), Err(CheckpointError::InvalidDigest));
    }
    let mut absent = prepared();
    absent.before = BeforeImage::Present(version("nope", 16));
    assert_eq!(absent.validate(), Err(CheckpointError::InvalidDigest));
}

// ---------------------------------------------------------------------------
// The per-file bound: exactly MAX_CHECKPOINT_FILE_BYTES, never one more
// ---------------------------------------------------------------------------

#[test]
fn a_recorded_version_admits_exactly_max_file_bytes_and_refuses_one_more() {
    let mut record = prepared();
    record.before = BeforeImage::Present(version(BEFORE_SHA256, MAX_CHECKPOINT_FILE_BYTES));
    record
        .validate()
        .expect("exactly the 8 MiB workspace IO ceiling fits the bound");
    record.before = BeforeImage::Present(version(BEFORE_SHA256, MAX_CHECKPOINT_FILE_BYTES + 1));
    assert_eq!(
        record.validate(),
        Err(CheckpointError::VersionTooLarge(
            MAX_CHECKPOINT_FILE_BYTES + 1
        ))
    );
    let mut after = committed();
    after.after = Some(version(AFTER_SHA256, MAX_CHECKPOINT_FILE_BYTES + 1));
    assert_eq!(
        after.validate(),
        Err(CheckpointError::VersionTooLarge(
            MAX_CHECKPOINT_FILE_BYTES + 1
        ))
    );
}
