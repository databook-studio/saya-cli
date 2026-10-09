use super::*;

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
