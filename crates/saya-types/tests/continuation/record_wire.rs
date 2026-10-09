use super::*;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// The bounds pin their scale, so a loosened bound is visible in the diff
// ---------------------------------------------------------------------------

#[test]
fn the_bounds_pin_their_scale() {
    assert_eq!(CONTINUATION_VERSION, 1);
    assert_eq!(MAX_CONTINUATION_BYTES, 32 * 1024);
    assert_eq!(MAX_NARRATIVE_BYTES, 8 * 1024);
    assert_eq!(MAX_USER_NOTES_BYTES, 8 * 1024);
    assert_eq!(MAX_FILE_FACTS, 64);
    assert_eq!(MAX_CHECK_FACTS, 32);
    assert_eq!(MAX_FACT_PATH_BYTES, 512);
    assert_eq!(MAX_CHECK_LABEL_CHARS, 200);
}

// ---------------------------------------------------------------------------
// Valid records round-trip
// ---------------------------------------------------------------------------

#[test]
fn a_valid_minimal_record_round_trips() {
    let record = minimal();
    let back = SessionContinuationV1::from_json_bytes(&record.to_json_bytes().unwrap()).unwrap();
    assert_eq!(back, record);
}

#[test]
fn a_valid_full_record_round_trips() {
    let record = full();
    let bytes = bytes_of(&record);
    let back = SessionContinuationV1::from_json_bytes(&bytes).unwrap();
    assert_eq!(back, record);
    assert_eq!(record.to_json_bytes().unwrap(), bytes);
}

#[test]
fn an_absent_continuation_deserializes_as_none() {
    let missing: Wrapped = serde_json::from_str("{}").unwrap();
    assert_eq!(missing.continuation, None);
    let null: Wrapped = serde_json::from_str(r#"{"continuation":null}"#).unwrap();
    assert_eq!(null.continuation, None);
}

#[derive(Serialize, Deserialize)]
struct Wrapped {
    #[serde(default)]
    continuation: Option<SessionContinuationV1>,
}

#[test]
fn an_embedded_continuation_comes_back_whole() {
    let record = full();
    let json = serde_json::to_string(&Wrapped {
        continuation: Some(record.clone()),
    })
    .unwrap();
    let back: Wrapped = serde_json::from_str(&json).unwrap();
    assert_eq!(back.continuation, Some(record));
}

// ---------------------------------------------------------------------------
// The version gate: unknown versions are refused, never ignored
// ---------------------------------------------------------------------------

#[test]
fn an_unknown_version_is_refused_never_ignored() {
    let mut record = minimal();
    record.version = 2;
    assert_eq!(
        SessionContinuationV1::from_json_bytes(&bytes_of(&record)),
        Err(ContinuationError::UnsupportedVersion(2))
    );
    record.version = 0;
    assert_eq!(
        SessionContinuationV1::from_json_bytes(&bytes_of(&record)),
        Err(ContinuationError::UnsupportedVersion(0))
    );
    record.version = 2;
    assert_eq!(
        record.validate(),
        Err(ContinuationError::UnsupportedVersion(2))
    );
}

// ---------------------------------------------------------------------------
// Unknown fields are refused at every level
// ---------------------------------------------------------------------------

#[test]
fn unknown_fields_are_refused() {
    let top = format!(
        r#"{{"version":1,"source":{{"through_turn":1,"source_sha256":"{VALID_SHA256}"}},"files":[],"checks":[],"bonus":1}}"#
    );
    assert_eq!(
        SessionContinuationV1::from_json_bytes(top.as_bytes()),
        Err(ContinuationError::Malformed)
    );
    let nested = format!(
        r#"{{"version":1,"source":{{"through_turn":1,"source_sha256":"{VALID_SHA256}","extra":1}},"files":[],"checks":[]}}"#
    );
    assert_eq!(
        SessionContinuationV1::from_json_bytes(nested.as_bytes()),
        Err(ContinuationError::Malformed)
    );
}
