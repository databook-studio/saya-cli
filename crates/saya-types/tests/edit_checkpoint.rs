//! Contract tests for edit checkpoint records and their recovery policy in
//! `saya_types`.
//!
//! A checkpoint is opt-in private recovery data for direct file-tool edits,
//! read back from storage — model-adjacent, untrusted input — so `validate()`
//! re-checks every bound no matter how the record arrived: the version, the
//! digests, the file sizes, the path shape, the timestamps, and the
//! state/after-image identity rules. It must be impossible to represent a
//! subprocess, network, scratch, or database rollback as a checkpoint, restore
//! is only eligible for a committed edit whose post-edit digest still matches
//! the file, and a quota refusal happens before mutation without evicting
//! history.

use saya_types::{
    BeforeImage, CHECKPOINT_VERSION, CheckpointError, CheckpointId, CheckpointPolicy,
    CheckpointRefusal, CheckpointState, CheckpointUsage, EditCheckpoint, EditSource, FileVersion,
    MAX_CHECKPOINT_FILE_BYTES, MAX_CHECKPOINT_PATH_BYTES, MAX_CHECKPOINT_RECORD_BYTES,
    MAX_CHECKPOINT_SESSION_BYTES, MAX_CHECKPOINTS_PER_SESSION, RestoreEligibility, admit,
    restore_eligibility,
};

const BEFORE_SHA256: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const AFTER_SHA256: &str = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
const ID: &str = "ck-00112233445566778899aabbccddeeff";
const SESSION: &str = "sess-1";
const CREATED: i64 = 1_700_000_000_000;

fn id() -> CheckpointId {
    CheckpointId::parse(ID).unwrap()
}

fn version(sha: &str, size: u64) -> FileVersion {
    FileVersion {
        sha256: sha.to_string(),
        size,
    }
}

fn record(state: CheckpointState, before: BeforeImage) -> EditCheckpoint {
    let after = (state == CheckpointState::Committed).then(|| version(AFTER_SHA256, 2048));
    EditCheckpoint {
        version: CHECKPOINT_VERSION,
        id: id(),
        session_id: SESSION.to_string(),
        path: "src/main.rs".to_string(),
        source: EditSource::WorkspaceEdit,
        state,
        before,
        after,
        created_unix_ms: CREATED,
        updated_unix_ms: CREATED,
    }
}

fn prepared() -> EditCheckpoint {
    record(
        CheckpointState::Prepared,
        BeforeImage::Present(version(BEFORE_SHA256, 1024)),
    )
}

fn committed() -> EditCheckpoint {
    record(
        CheckpointState::Committed,
        BeforeImage::Present(version(BEFORE_SHA256, 1024)),
    )
}

fn uncertain() -> EditCheckpoint {
    record(
        CheckpointState::Uncertain,
        BeforeImage::Present(version(BEFORE_SHA256, 1024)),
    )
}

fn create_committed() -> EditCheckpoint {
    record(CheckpointState::Committed, BeforeImage::Absent)
}

fn bytes_of(record: &EditCheckpoint) -> Vec<u8> {
    serde_json::to_vec(record).unwrap()
}

fn usage(edits: u32, retained_bytes: u64) -> CheckpointUsage {
    CheckpointUsage {
        edits,
        retained_bytes,
    }
}

// ---------------------------------------------------------------------------
// The bounds pin their scale, so a loosened bound is visible in the diff
// ---------------------------------------------------------------------------

#[test]
fn the_bounds_pin_their_scale() {
    assert_eq!(CHECKPOINT_VERSION, 1);
    assert_eq!(MAX_CHECKPOINT_RECORD_BYTES, 4 * 1024);
    assert_eq!(MAX_CHECKPOINT_FILE_BYTES, 8 * 1024 * 1024);
    assert_eq!(MAX_CHECKPOINT_SESSION_BYTES, 64 * 1024 * 1024);
    assert_eq!(MAX_CHECKPOINTS_PER_SESSION, 128);
    assert_eq!(MAX_CHECKPOINT_PATH_BYTES, 512);
}

// ---------------------------------------------------------------------------
// Valid records round-trip
// ---------------------------------------------------------------------------

#[test]
fn a_valid_record_in_each_state_round_trips() {
    for make in [prepared, committed, uncertain] {
        let record = make();
        let bytes = record.to_json_bytes().unwrap();
        assert_eq!(EditCheckpoint::from_json_bytes(&bytes).unwrap(), record);
    }
}

#[test]
fn a_create_edit_round_trips_with_an_absent_before_image() {
    let record = create_committed();
    let bytes = record.to_json_bytes().unwrap();
    assert_eq!(EditCheckpoint::from_json_bytes(&bytes).unwrap(), record);
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["before"], "absent");
    assert!(json.get("after").is_some());
}

// ---------------------------------------------------------------------------
// The version gate: unknown versions are refused, never downgraded
// ---------------------------------------------------------------------------

#[test]
fn an_unknown_version_is_refused_never_ignored() {
    for v in [0u32, 2] {
        let mut record = prepared();
        record.version = v;
        assert_eq!(
            EditCheckpoint::from_json_bytes(&bytes_of(&record)),
            Err(CheckpointError::UnsupportedVersion(v))
        );
        assert_eq!(
            record.validate(),
            Err(CheckpointError::UnsupportedVersion(v))
        );
    }
}

// ---------------------------------------------------------------------------
// Unknown fields and non-file sources are refused at every level
// ---------------------------------------------------------------------------

#[test]
fn unknown_fields_are_refused() {
    let top = format!(
        r#"{{"version":1,"id":"{ID}","session_id":"{SESSION}","path":"src/main.rs","source":"workspace_edit","state":"prepared","before":{{"present":{{"sha256":"{BEFORE_SHA256}","size":16}}}},"created_unix_ms":{CREATED},"updated_unix_ms":{CREATED},"bonus":1}}"#
    );
    assert_eq!(
        EditCheckpoint::from_json_bytes(top.as_bytes()),
        Err(CheckpointError::Malformed)
    );
    let nested = top.replace(r#""size":16"#, r#""size":16,"extra":1"#);
    assert_eq!(
        EditCheckpoint::from_json_bytes(nested.as_bytes()),
        Err(CheckpointError::Malformed)
    );
}

#[test]
fn a_subprocess_or_shell_source_is_refused_never_ignored() {
    let template = |source: &str| {
        format!(
            r#"{{"version":1,"id":"{ID}","session_id":"{SESSION}","path":"src/main.rs","source":"{source}","state":"prepared","before":{{"present":{{"sha256":"{BEFORE_SHA256}","size":16}}}},"created_unix_ms":{CREATED},"updated_unix_ms":{CREATED}}}"#
        )
    };
    for source in ["subprocess", "shell"] {
        assert_eq!(
            EditCheckpoint::from_json_bytes(template(source).as_bytes()),
            Err(CheckpointError::Malformed),
            "a `{source}` effect is never representable as a checkpoint"
        );
    }
    EditCheckpoint::from_json_bytes(template("workspace_edit").as_bytes())
        .expect("the file-tool twin is the only representable source");
}

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

// ---------------------------------------------------------------------------
// The path bound: one relative file, canonical shape, no control characters
// ---------------------------------------------------------------------------

#[test]
fn a_path_that_is_not_one_relative_file_is_refused() {
    for path in [
        "",
        "/abs/main.rs",
        "C:\\abs\\main.rs",
        "../escape.rs",
        "a//b.rs",
        "dir/",
        "bad\u{0}path",
    ] {
        let mut record = prepared();
        record.path = path.to_string();
        assert_eq!(record.validate(), Err(CheckpointError::InvalidPath));
    }
}

#[test]
fn a_path_admits_exactly_max_path_bytes_and_refuses_one_more() {
    let mut record = prepared();
    record.path = "a".repeat(MAX_CHECKPOINT_PATH_BYTES);
    record
        .validate()
        .expect("exactly 512 bytes fits the path bound");
    record.path = "a".repeat(MAX_CHECKPOINT_PATH_BYTES + 1);
    assert_eq!(
        record.validate(),
        Err(CheckpointError::PathOversize(MAX_CHECKPOINT_PATH_BYTES + 1))
    );
}

// ---------------------------------------------------------------------------
// Timestamps: created_unix_ms >= 0 and updated_unix_ms >= created_unix_ms
// ---------------------------------------------------------------------------

#[test]
fn timestamps_must_be_ordered_and_non_negative() {
    let mut record = committed();
    record.created_unix_ms = -1;
    assert_eq!(record.validate(), Err(CheckpointError::NegativeTimestamp));
    record.created_unix_ms = CREATED;
    record.updated_unix_ms = CREATED - 1;
    assert_eq!(record.validate(), Err(CheckpointError::TimestampOrder));
    record.updated_unix_ms = CREATED;
    record.validate().expect("equal timestamps are ordered");
}

// ---------------------------------------------------------------------------
// The checkpoint id: `ck-` + 32 lowercase hex, on the type and on the wire
// ---------------------------------------------------------------------------

#[test]
fn an_id_must_be_ck_prefixed_lowercase_hex() {
    for bad in [
        "",
        "00112233445566778899aabbccddeeff",
        "ck-00112233445566778899aabbccddeef",
        "ck-00112233445566778899aabbccddeefff",
        "ck-00112233445566778899aabbccddeefg",
        "ck-00112233445566778899AABBCCDDEEFF",
    ] {
        assert_eq!(
            CheckpointId::parse(bad),
            Err(CheckpointError::InvalidId),
            "id {bad:?} must be refused"
        );
    }
    assert_eq!(id().as_str(), ID);
    let bad_wire = format!(
        r#"{{"version":1,"id":"nope","session_id":"{SESSION}","path":"src/main.rs","source":"workspace_edit","state":"prepared","before":{{"present":{{"sha256":"{BEFORE_SHA256}","size":16}}}},"created_unix_ms":{CREATED},"updated_unix_ms":{CREATED}}}"#
    );
    assert_eq!(
        EditCheckpoint::from_json_bytes(bad_wire.as_bytes()),
        Err(CheckpointError::InvalidId)
    );
}

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

// ---------------------------------------------------------------------------
// Policy: opt-in, disabled by default, and never enabled silently
// ---------------------------------------------------------------------------

#[test]
fn recovery_is_disabled_by_default_and_opt_in() {
    assert!(!CheckpointPolicy::default().enabled);
    let empty: CheckpointPolicy = serde_json::from_str("{}").unwrap();
    assert!(!empty.enabled);
    let explicit: CheckpointPolicy = serde_json::from_str(r#"{"enabled":true}"#).unwrap();
    assert!(explicit.enabled);
    let back: CheckpointPolicy =
        serde_json::from_str(&serde_json::to_string(&empty).unwrap()).unwrap();
    assert_eq!(back, empty);
}

// ---------------------------------------------------------------------------
// The serialized record bound: enforced before any parsing, and on the way out
// ---------------------------------------------------------------------------

#[test]
fn the_record_byte_cap_is_enforced_before_any_parsing() {
    let over = vec![b' '; MAX_CHECKPOINT_RECORD_BYTES + 1];
    assert_eq!(
        EditCheckpoint::from_json_bytes(&over),
        Err(CheckpointError::Oversize(MAX_CHECKPOINT_RECORD_BYTES + 1))
    );
    let record = prepared();
    let bytes = bytes_of(&record);
    assert!(bytes.len() <= MAX_CHECKPOINT_RECORD_BYTES);
    EditCheckpoint::from_json_bytes(&bytes).expect("a real record fits the cap");
}

#[test]
fn to_json_bytes_never_serializes_an_invalid_record() {
    let mut record = committed();
    record.after = None;
    assert_eq!(
        record.to_json_bytes(),
        Err(CheckpointError::CommittedRequiresAfter)
    );
}
