//! Contract tests for the session continuation record in `saya_types`.
//!
//! A continuation is carried inside a session record and read back on resume —
//! model-adjacent, untrusted input — so `validate()` re-checks every bound no
//! matter how the record arrived, and an unknown version, unknown field, or
//! unprovenanced fact is refused, never ignored. Absence stays representable:
//! a legacy record without the field deserializes as `None`.

use saya_types::{
    CONTINUATION_VERSION, CheckFact, CheckOutcome, ContinuationError, FactProvenance, FileFact,
    GeneratedNarrative, MAX_CHECK_FACTS, MAX_CHECK_LABEL_CHARS, MAX_CONTINUATION_BYTES,
    MAX_FACT_PATH_BYTES, MAX_FILE_FACTS, MAX_NARRATIVE_BYTES, MAX_USER_NOTES_BYTES,
    NarrativeSource, SessionContinuationV1, SourceBoundary,
};
use serde::{Deserialize, Serialize};

const VALID_SHA256: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const TOOL: &str = "saya_test_tool";
const OBSERVED: i64 = 1_700_000_000_000;

fn boundary() -> SourceBoundary {
    SourceBoundary {
        through_turn: 12,
        source_sha256: VALID_SHA256.to_string(),
        workspace_root: Some("/work/saya".to_string()),
        profile: Some("main".to_string()),
    }
}

fn file(path: &str) -> FileFact {
    FileFact {
        path: path.to_string(),
        sha256: Some(VALID_SHA256.to_string()),
        observed_unix_ms: OBSERVED,
        provenance: FactProvenance::ToolObserved {
            tool: TOOL.to_string(),
        },
    }
}

fn check(label: &str) -> CheckFact {
    CheckFact {
        label: label.to_string(),
        outcome: CheckOutcome::Passed,
        observed_unix_ms: OBSERVED,
        provenance: FactProvenance::ToolObserved {
            tool: TOOL.to_string(),
        },
    }
}

fn narrative(text: &str) -> Option<GeneratedNarrative> {
    Some(GeneratedNarrative {
        text: text.to_string(),
        source: NarrativeSource::Summarizer,
    })
}

fn minimal() -> SessionContinuationV1 {
    SessionContinuationV1 {
        version: CONTINUATION_VERSION,
        source: boundary(),
        narrative: None,
        user_notes: None,
        files: Vec::new(),
        checks: Vec::new(),
    }
}

fn full() -> SessionContinuationV1 {
    SessionContinuationV1 {
        version: CONTINUATION_VERSION,
        source: boundary(),
        narrative: narrative("profiled the tables\nnext: check the indexes"),
        user_notes: Some("keep the schema cache in mind".to_string()),
        files: vec![file("src/main.rs"), file("crates/saya-types/src/lib.rs")],
        checks: vec![check("cargo test"), check("cargo clippy")],
    }
}

fn bytes_of(record: &SessionContinuationV1) -> Vec<u8> {
    serde_json::to_vec(record).unwrap()
}

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

// ---------------------------------------------------------------------------
// Provenance: only a tool observation certifies a fact
// ---------------------------------------------------------------------------

#[test]
fn a_model_asserted_provenance_is_refused_not_ignored() {
    let json = format!(
        r#"{{"version":1,"source":{{"through_turn":1,"source_sha256":"{VALID_SHA256}"}},"files":[{{"path":"src/main.rs","observed_unix_ms":1700000000000,"provenance":{{"kind":"model_asserted"}}}}],"checks":[]}}"#
    );
    assert_eq!(
        SessionContinuationV1::from_json_bytes(json.as_bytes()),
        Err(ContinuationError::Malformed)
    );
    let tool = json.replace(
        r#""kind":"model_asserted""#,
        r#""kind":"tool_observed","tool":"saya""#,
    );
    SessionContinuationV1::from_json_bytes(tool.as_bytes())
        .expect("the tool-observed twin is the grant-free shape and parses");
}

#[test]
fn tool_observed_provenance_is_tagged_kind() {
    let json = serde_json::to_string(&file("p").provenance).unwrap();
    assert_eq!(json, r#"{"kind":"tool_observed","tool":"saya_test_tool"}"#);
}

#[test]
fn an_unname_shaped_tool_is_refused() {
    let mut record = minimal();
    record.files = vec![file("src/main.rs")];
    record.files[0].provenance = FactProvenance::ToolObserved {
        tool: String::new(),
    };
    assert_eq!(record.validate(), Err(ContinuationError::InvalidToolName));
    record.files[0].provenance = FactProvenance::ToolObserved {
        tool: "bad tool".to_string(),
    };
    assert_eq!(record.validate(), Err(ContinuationError::InvalidToolName));
    record.files[0].provenance = FactProvenance::ToolObserved {
        tool: "tool\u{0}".to_string(),
    };
    assert_eq!(record.validate(), Err(ContinuationError::InvalidToolName));
}

// ---------------------------------------------------------------------------
// The narrative bound: exactly MAX_NARRATIVE_BYTES, never empty, only
// newline and tab as control characters
// ---------------------------------------------------------------------------

#[test]
fn the_narrative_admits_exactly_max_bytes_and_refuses_one_more() {
    let mut record = minimal();
    record.narrative = narrative(&"a".repeat(MAX_NARRATIVE_BYTES));
    record
        .validate()
        .expect("exactly 8 KiB of narrative is inside the bound");

    record.narrative = narrative(&"a".repeat(MAX_NARRATIVE_BYTES + 1));
    assert_eq!(
        record.validate(),
        Err(ContinuationError::NarrativeOversize(
            MAX_NARRATIVE_BYTES + 1
        ))
    );
}

#[test]
fn the_narrative_must_not_be_empty() {
    let mut record = minimal();
    record.narrative = narrative("");
    assert_eq!(record.validate(), Err(ContinuationError::NarrativeEmpty));
}

#[test]
fn narrative_and_notes_allow_newline_and_tab_only() {
    let mut record = minimal();
    record.narrative = narrative("line one\nline two\tindented");
    record.user_notes = Some("note\nmore\ttabs".to_string());
    record
        .validate()
        .expect("newline and tab are allowed prose control");

    record.narrative = narrative("bell\u{0007}");
    assert_eq!(
        record.validate(),
        Err(ContinuationError::NarrativeControlCharacter)
    );
    record.narrative = narrative("carriage\rreturn");
    assert_eq!(
        record.validate(),
        Err(ContinuationError::NarrativeControlCharacter)
    );
    record.narrative = None;
    record.user_notes = Some("carriage\rreturn".to_string());
    assert_eq!(
        record.validate(),
        Err(ContinuationError::UserNotesControlCharacter)
    );
    record.user_notes = Some("nul\u{0}byte".to_string());
    assert_eq!(
        record.validate(),
        Err(ContinuationError::UserNotesControlCharacter)
    );
}

// ---------------------------------------------------------------------------
// The user-notes bound: exactly MAX_USER_NOTES_BYTES, never empty
// ---------------------------------------------------------------------------

#[test]
fn the_user_notes_admit_exactly_max_bytes_and_refuse_one_more() {
    let mut record = minimal();
    record.user_notes = Some("a".repeat(MAX_USER_NOTES_BYTES));
    record
        .validate()
        .expect("exactly 8 KiB of notes is inside the bound");

    record.user_notes = Some("a".repeat(MAX_USER_NOTES_BYTES + 1));
    assert_eq!(
        record.validate(),
        Err(ContinuationError::UserNotesOversize(
            MAX_USER_NOTES_BYTES + 1
        ))
    );
}

#[test]
fn user_notes_must_not_be_empty() {
    let mut record = minimal();
    record.user_notes = Some(String::new());
    assert_eq!(record.validate(), Err(ContinuationError::UserNotesEmpty));
}

// ---------------------------------------------------------------------------
// The fact-count bounds: exactly MAX_FILE_FACTS / MAX_CHECK_FACTS
// ---------------------------------------------------------------------------

#[test]
fn the_file_count_admits_exactly_max_and_refuses_one_more() {
    let mut record = minimal();
    record.files = (0..MAX_FILE_FACTS)
        .map(|i| file(&format!("src/f{i}.rs")))
        .collect();
    record
        .validate()
        .expect("64 file facts is inside the bound");

    record.files = (0..=MAX_FILE_FACTS)
        .map(|i| file(&format!("src/f{i}.rs")))
        .collect();
    assert_eq!(record.validate(), Err(ContinuationError::TooManyFiles(65)));
}

#[test]
fn the_check_count_admits_exactly_max_and_refuses_one_more() {
    let mut record = minimal();
    record.checks = (0..MAX_CHECK_FACTS)
        .map(|i| check(&format!("check {i}")))
        .collect();
    record
        .validate()
        .expect("32 check facts is inside the bound");

    record.checks = (0..=MAX_CHECK_FACTS)
        .map(|i| check(&format!("check {i}")))
        .collect();
    assert_eq!(record.validate(), Err(ContinuationError::TooManyChecks(33)));
}

// ---------------------------------------------------------------------------
// The path bound: exactly MAX_FACT_PATH_BYTES, then the path shape
// ---------------------------------------------------------------------------

#[test]
fn the_fact_path_admits_exactly_max_bytes_and_refuses_one_more() {
    let mut record = minimal();
    record.files = vec![file(&"a".repeat(MAX_FACT_PATH_BYTES))];
    record
        .validate()
        .expect("a 512-byte relative path is inside the bound");

    record.files = vec![file(&"a".repeat(MAX_FACT_PATH_BYTES + 1))];
    assert_eq!(
        record.validate(),
        Err(ContinuationError::FactPathOversize(513))
    );
}

#[test]
fn the_serialized_total_is_capped_with_many_max_size_files() {
    let mut record = minimal();
    record.files = (0..MAX_FILE_FACTS)
        .map(|i| file(&format!("{i:02}{}", "a".repeat(MAX_FACT_PATH_BYTES - 2))))
        .collect();
    let bytes = bytes_of(&record);
    assert!(
        bytes.len() > MAX_CONTINUATION_BYTES,
        "the witness must actually exceed the cap: {} bytes",
        bytes.len()
    );
    assert_eq!(
        record.validate(),
        Err(ContinuationError::Oversize(bytes.len()))
    );
    assert_eq!(
        SessionContinuationV1::from_json_bytes(&bytes),
        Err(ContinuationError::Oversize(bytes.len()))
    );
}

#[test]
fn traversal_absolute_and_non_canonical_paths_are_refused() {
    for bad in [
        "..",
        "../escape",
        "src/../escape",
        "/etc/passwd",
        "\\Windows\\evil",
        "C:\\Users\\evil",
        "C:/Users/evil",
        "src/",
        "a//b",
    ] {
        let mut record = minimal();
        record.files = vec![file(bad)];
        assert_eq!(
            record.validate(),
            Err(ContinuationError::InvalidFactPath),
            "path {bad:?} must be refused"
        );
    }
}

#[test]
fn control_characters_in_paths_are_refused() {
    let mut record = minimal();
    record.files = vec![file("src/main.rs\0")];
    assert_eq!(record.validate(), Err(ContinuationError::InvalidFactPath));
    record.files = vec![file("src/bell\u{0007}.rs")];
    assert_eq!(record.validate(), Err(ContinuationError::InvalidFactPath));
}

#[test]
fn duplicate_file_paths_are_refused() {
    let mut record = minimal();
    record.files = vec![file("src/main.rs"), file("src/main.rs")];
    assert_eq!(record.validate(), Err(ContinuationError::DuplicateFilePath));
}

// ---------------------------------------------------------------------------
// Digest shape: 64 lowercase hex characters, on the source and on facts
// ---------------------------------------------------------------------------

#[test]
fn a_bad_source_digest_is_refused() {
    for bad in [
        "",
        "abc",
        &"g".repeat(64),
        &VALID_SHA256.to_uppercase(),
        &format!("{VALID_SHA256}0"),
    ] {
        let mut record = minimal();
        record.source.source_sha256 = bad.to_string();
        assert_eq!(
            record.validate(),
            Err(ContinuationError::InvalidDigest),
            "digest {bad:?} must be refused"
        );
    }
}

#[test]
fn a_bad_file_digest_is_refused() {
    let mut record = minimal();
    record.files = vec![file("src/main.rs")];
    record.files[0].sha256 = Some("nope".to_string());
    assert_eq!(record.validate(), Err(ContinuationError::InvalidDigest));
    record.files[0].sha256 = Some(VALID_SHA256.to_uppercase());
    assert_eq!(record.validate(), Err(ContinuationError::InvalidDigest));
}

// ---------------------------------------------------------------------------
// Timestamps are unix-ms and never negative
// ---------------------------------------------------------------------------

#[test]
fn a_negative_observed_timestamp_is_refused() {
    let mut record = minimal();
    record.files = vec![file("src/main.rs")];
    record.files[0].observed_unix_ms = -1;
    assert_eq!(record.validate(), Err(ContinuationError::NegativeTimestamp));
    record.files[0].observed_unix_ms = OBSERVED;
    record.checks = vec![check("cargo test")];
    record.checks[0].observed_unix_ms = -1;
    assert_eq!(record.validate(), Err(ContinuationError::NegativeTimestamp));
}

// ---------------------------------------------------------------------------
// The check label bound: bounded like a task title
// ---------------------------------------------------------------------------

#[test]
fn the_check_label_admits_exactly_max_chars_and_refuses_one_more() {
    let mut record = minimal();
    record.checks = vec![check(&"a".repeat(MAX_CHECK_LABEL_CHARS))];
    record
        .validate()
        .expect("a 200-character label is inside the bound");

    record.checks = vec![check(&"a".repeat(MAX_CHECK_LABEL_CHARS + 1))];
    assert_eq!(record.validate(), Err(ContinuationError::InvalidCheckLabel));
}

#[test]
fn empty_and_control_character_labels_are_refused() {
    let mut record = minimal();
    record.checks = vec![check("")];
    assert_eq!(record.validate(), Err(ContinuationError::InvalidCheckLabel));
    record.checks = vec![check("   ")];
    assert_eq!(record.validate(), Err(ContinuationError::InvalidCheckLabel));
    record.checks = vec![check("label\u{0007}bell")];
    assert_eq!(record.validate(), Err(ContinuationError::InvalidCheckLabel));
}
