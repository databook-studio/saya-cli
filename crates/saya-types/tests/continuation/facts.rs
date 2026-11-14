use super::*;

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
