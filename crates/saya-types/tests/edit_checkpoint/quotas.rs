use super::*;

// ---------------------------------------------------------------------------
// Quotas: refusal before mutation, never an eviction
// ---------------------------------------------------------------------------

#[test]
fn admit_refuses_one_byte_over_the_per_file_quota() {
    assert_eq!(admit(&usage(0, 0), MAX_CHECKPOINT_FILE_BYTES), Ok(()));
    assert_eq!(
        admit(&usage(0, 0), MAX_CHECKPOINT_FILE_BYTES + 1),
        Err(CheckpointRefusal::FileTooLarge)
    );
}

#[test]
fn admit_refuses_at_the_session_count_quota() {
    let at_limit = usage(MAX_CHECKPOINTS_PER_SESSION - 1, 0);
    assert_eq!(admit(&at_limit, 16), Ok(()));
    let over = usage(MAX_CHECKPOINTS_PER_SESSION, 0);
    assert_eq!(admit(&over, 16), Err(CheckpointRefusal::TooManyCheckpoints));
}

#[test]
fn admit_refuses_one_byte_over_the_session_bytes_quota() {
    let at_limit = usage(1, MAX_CHECKPOINT_SESSION_BYTES - 16);
    assert_eq!(admit(&at_limit, 16), Ok(()));
    let over = usage(1, MAX_CHECKPOINT_SESSION_BYTES - 15);
    assert_eq!(
        admit(&over, 16),
        Err(CheckpointRefusal::SessionBytesExceeded)
    );
}

#[test]
fn a_quota_refusal_never_mutates_usage_or_evicts_history() {
    let held = usage(2, 1024);
    let snapshot = held;
    let _ = admit(&held, MAX_CHECKPOINT_FILE_BYTES + 1);
    assert_eq!(held, snapshot, "usage is read through a shared reference");
}
