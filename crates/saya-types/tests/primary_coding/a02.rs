use saya_types::{
    AgentId, CheckReceiptV1, CodingCheckOutcome, CodingEventError, CodingEventKind, CodingEventV1,
    CodingResultOutcome, InvocationId, JobId, MAX_RECEIPT_PATH_BYTES, MAX_RECEIPT_PATH_SET_BYTES,
    MAX_RECEIPT_PATHS, MAX_RECEIPT_REFERENCE_BYTES, ReceiptError, ResultReceiptV1, Sha256Digest,
};

const SOURCE_SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const ARGV_SHA: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn assert_json_matches(actual: &[u8], expected: &str) {
    let actual: serde_json::Value = serde_json::from_slice(actual).unwrap();
    let expected: serde_json::Value = serde_json::from_str(expected).unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn event_wire_version_compatibility() {
    let supported = r#"{"version":1,"sequence":1,"job_id":"job-1","agent_id":"agent-1","invocation_id":"inv-1","event":{"type":"state_changed","state":"running"}}"#;
    let event = CodingEventV1::from_json_bytes(supported.as_bytes()).unwrap();
    assert_json_matches(&event.to_json_bytes().unwrap(), supported);
    assert_eq!(event.sequence().get(), 1);
    assert_eq!(event.job_id().as_str(), "job-1");
    assert_eq!(
        event.event(),
        &CodingEventKind::StateChanged {
            state: saya_types::JobState::Running
        }
    );

    let future = r#"{"version":2,"sequence":1,"job_id":"job-1","event":{"type":"state_changed","state":"running"},"capabilities":{"workspace_write":true}}"#;
    assert_eq!(
        CodingEventV1::from_json_bytes(future.as_bytes()),
        Err(CodingEventError::UnsupportedVersion(2))
    );

    let extra_authority = r#"{"version":1,"sequence":1,"job_id":"job-1","event":{"type":"state_changed","state":"running"},"capabilities":{"workspace_write":true}}"#;
    assert_eq!(
        CodingEventV1::from_json_bytes(extra_authority.as_bytes()),
        Err(CodingEventError::Malformed)
    );
    let zero_sequence = r#"{"version":1,"sequence":0,"job_id":"job-1","event":{"type":"state_changed","state":"running"}}"#;
    assert_eq!(
        CodingEventV1::from_json_bytes(zero_sequence.as_bytes()),
        Err(CodingEventError::Malformed)
    );
    let result_without_child = format!(
        r#"{{"version":1,"sequence":2,"job_id":"job-1","event":{{"type":"result_reported","source_sha256":"{SOURCE_SHA}","outcome":"completed"}}}}"#
    );
    assert_eq!(
        CodingEventV1::from_json_bytes(result_without_child.as_bytes()),
        Err(CodingEventError::InvalidIdentity)
    );
    let result_event = format!(
        r#"{{"version":1,"sequence":2,"job_id":"job-1","agent_id":"agent-1","invocation_id":"inv-1","event":{{"type":"result_reported","source_sha256":"{SOURCE_SHA}","outcome":"uncertain"}}}}"#
    );
    let event = CodingEventV1::from_json_bytes(result_event.as_bytes()).unwrap();
    assert_json_matches(&event.to_json_bytes().unwrap(), &result_event);
    assert!(matches!(
        event.event(),
        CodingEventKind::ResultReported {
            outcome: CodingResultOutcome::Uncertain,
            ..
        }
    ));
    let invalid_digest = result_event.replace(SOURCE_SHA, "invalid");
    assert_eq!(
        CodingEventV1::from_json_bytes(invalid_digest.as_bytes()),
        Err(CodingEventError::InvalidDigest)
    );
}

fn result_wire(source: &str, path: &str) -> String {
    format!(
        r#"{{"version":1,"job_id":"job-1","agent_id":"agent-1","invocation_id":"inv-1","source_sha256":"{source}","paths":[{path:?}],"outcome":"completed"}}"#
    )
}

#[test]
fn result_receipt_rejects_bad_digest_and_escaping_paths() {
    let bad_digest = result_wire("not-sha256", "src/main.rs");
    assert_eq!(
        ResultReceiptV1::from_json_bytes(bad_digest.as_bytes()),
        Err(ReceiptError::InvalidDigest)
    );

    for path in ["/etc/passwd", "../outside.rs", "C:\\private\\key"] {
        let wire = result_wire(SOURCE_SHA, path);
        assert_eq!(
            ResultReceiptV1::from_json_bytes(wire.as_bytes()),
            Err(ReceiptError::InvalidPath)
        );
    }

    let good = result_wire(SOURCE_SHA, "crates/saya/src/lib.rs");
    let receipt = ResultReceiptV1::from_json_bytes(good.as_bytes()).unwrap();
    assert_json_matches(&receipt.to_json_bytes().unwrap(), &good);
    assert_eq!(receipt.source_sha256().as_str(), SOURCE_SHA);
    assert_eq!(receipt.outcome(), CodingResultOutcome::Completed);
    for (wire_outcome, expected) in [
        ("completed", CodingResultOutcome::Completed),
        ("failed", CodingResultOutcome::Failed),
        ("cancelled", CodingResultOutcome::Cancelled),
        ("uncertain", CodingResultOutcome::Uncertain),
    ] {
        let wire = good.replace(
            "\"outcome\":\"completed\"",
            &format!("\"outcome\":\"{wire_outcome}\""),
        );
        let receipt = ResultReceiptV1::from_json_bytes(wire.as_bytes()).unwrap();
        assert_json_matches(&receipt.to_json_bytes().unwrap(), &wire);
        assert_eq!(receipt.outcome(), expected);
    }

    let future = good.replace("\"version\":1", "\"version\":2");
    assert_eq!(
        ResultReceiptV1::from_json_bytes(future.as_bytes()),
        Err(ReceiptError::UnsupportedVersion(2))
    );
    let extra = good.replace(
        "\"outcome\":\"completed\"}",
        "\"outcome\":\"completed\",\"capabilities\":{}}",
    );
    assert_eq!(
        ResultReceiptV1::from_json_bytes(extra.as_bytes()),
        Err(ReceiptError::Malformed)
    );
}

fn check_wire(outcome: &str) -> String {
    format!(
        r#"{{"version":1,"job_id":"job-1","source_sha256":"{SOURCE_SHA}","command_id":"workspace-tests","argv_sha256":"{ARGV_SHA}","toolchain":"rust-1.88","environment_profile":"macos-arm64","outcome":{outcome}}}"#
    )
}

#[test]
fn check_receipt_keeps_unknown_and_uncertain_outcomes_explicit() {
    for (wire_outcome, expected) in [
        (
            r#"{"status":"known_exit","code":3}"#,
            CodingCheckOutcome::KnownExit { code: 3 },
        ),
        (r#"{"status":"unknown"}"#, CodingCheckOutcome::Unknown),
        (r#"{"status":"uncertain"}"#, CodingCheckOutcome::Uncertain),
    ] {
        let wire = check_wire(wire_outcome);
        let receipt = CheckReceiptV1::from_json_bytes(wire.as_bytes()).unwrap();
        assert_json_matches(&receipt.to_json_bytes().unwrap(), &wire);
        assert_eq!(receipt.outcome(), expected);
    }

    let missing = check_wire("null").replace(r#","outcome":null"#, "");
    assert_eq!(
        CheckReceiptV1::from_json_bytes(missing.as_bytes()),
        Err(ReceiptError::Malformed)
    );
    let invented_success = check_wire(r#"{"status":"success"}"#);
    assert_eq!(
        CheckReceiptV1::from_json_bytes(invented_success.as_bytes()),
        Err(ReceiptError::Malformed)
    );

    let future = check_wire(r#"{"status":"unknown"}"#).replace("\"version\":1", "\"version\":2");
    assert_eq!(
        CheckReceiptV1::from_json_bytes(future.as_bytes()),
        Err(ReceiptError::UnsupportedVersion(2))
    );
    let extra = check_wire(r#"{"status":"unknown"}"#).replace(
        "\"environment_profile\":\"macos-arm64\",",
        "\"environment_profile\":\"macos-arm64\",\"capabilities\":{},",
    );
    assert_eq!(
        CheckReceiptV1::from_json_bytes(extra.as_bytes()),
        Err(ReceiptError::Malformed)
    );
}

#[test]
fn receipt_path_and_reference_bounds_are_enforced() {
    let source = Sha256Digest::parse(SOURCE_SHA).unwrap();
    let paths: Vec<String> = (0..64)
        .map(|index| format!("{index:02}-{}", "a".repeat(MAX_RECEIPT_PATH_BYTES - 3)))
        .collect();
    assert_eq!(
        paths.iter().map(String::len).sum::<usize>(),
        MAX_RECEIPT_PATH_SET_BYTES
    );
    let accepted = result_with_paths(paths.clone());
    assert!(accepted.is_ok());

    let too_many = vec!["src/file.rs".to_string(); MAX_RECEIPT_PATHS + 1];
    assert_eq!(result_with_paths(too_many), Err(ReceiptError::TooManyPaths));

    let too_long = vec!["a".repeat(MAX_RECEIPT_PATH_BYTES + 1)];
    assert_eq!(result_with_paths(too_long), Err(ReceiptError::PathTooLong));

    let duplicate = vec!["src/main.rs".to_string(); 2];
    assert_eq!(
        result_with_paths(duplicate),
        Err(ReceiptError::DuplicatePath)
    );

    let aggregate_too_large: Vec<String> = (0..65)
        .map(|index| format!("{index:03}-{}", "a".repeat(MAX_RECEIPT_PATH_BYTES - 4)))
        .collect();
    assert_eq!(
        result_with_paths(aggregate_too_large),
        Err(ReceiptError::PathSetTooLarge)
    );

    let receipt = CheckReceiptV1::new(
        JobId::parse("job-1").unwrap(),
        source.clone(),
        "c".repeat(MAX_RECEIPT_REFERENCE_BYTES),
        Sha256Digest::parse(ARGV_SHA).unwrap(),
        "t".repeat(MAX_RECEIPT_REFERENCE_BYTES),
        "e".repeat(MAX_RECEIPT_REFERENCE_BYTES),
        CodingCheckOutcome::Unknown,
    );
    assert!(receipt.is_ok());
    assert_eq!(
        CheckReceiptV1::new(
            JobId::parse("job-1").unwrap(),
            source,
            "c".repeat(MAX_RECEIPT_REFERENCE_BYTES + 1),
            Sha256Digest::parse(ARGV_SHA).unwrap(),
            "rust-1.88".to_string(),
            "macos-arm64".to_string(),
            CodingCheckOutcome::Unknown,
        ),
        Err(ReceiptError::InvalidReference)
    );
}

fn result_with_paths(paths: Vec<String>) -> Result<ResultReceiptV1, ReceiptError> {
    ResultReceiptV1::new(
        JobId::parse("job-1").unwrap(),
        AgentId::parse("agent-1").unwrap(),
        InvocationId::parse("inv-1").unwrap(),
        Sha256Digest::parse(SOURCE_SHA).unwrap(),
        paths,
        CodingResultOutcome::Completed,
    )
}
