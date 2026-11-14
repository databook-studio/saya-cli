use super::*;

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
