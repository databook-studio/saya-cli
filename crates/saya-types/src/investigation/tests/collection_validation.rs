use super::*;

#[test]
fn validate_refuses_bad_parameter_spec_lists() {
    let mut definition = definition_with_parameters();
    definition
        .parameters
        .push(parameter_spec("region", ParamType::String));
    assert_eq!(
        definition.validate(),
        Err(InvestigationError::InvalidParameterSpec(
            ParamError::DuplicateName("region".to_owned())
        ))
    );
    definition.parameters = (0..=MAX_PARAMETERS)
        .map(|i| parameter_spec(&format!("p{i:02}"), ParamType::String))
        .collect();
    assert!(matches!(
        definition.validate(),
        Err(InvestigationError::InvalidParameterSpec(
            ParamError::TooManySpecs(33)
        ))
    ));
    definition.parameters = vec![parameter_spec("Region", ParamType::String)];
    assert!(matches!(
        definition.validate(),
        Err(InvestigationError::InvalidParameterSpec(
            ParamError::InvalidName(_)
        ))
    ));
}

#[test]
fn oversize_cap_still_applies_with_parameters() {
    let mut definition = definition_with_parameters();
    definition.description = Some("x".repeat(MAX_DESCRIPTION_BYTES));
    definition.objects = (0..MAX_OBJECTS)
        .map(|i| format!("{i:0>3}{}", "x".repeat(MAX_OBJECT_BYTES - 3)))
        .collect();
    definition.sql = "s".repeat(MAX_SQL_BYTES);
    definition
        .validate()
        .expect("every individual bound holds, yet the whole document cannot fit");
    assert!(matches!(
        definition.to_json_pretty(),
        Err(InvestigationError::Oversize(len)) if len > MAX_DEFINITION_BYTES
    ));
}

#[test]
fn objects_bounds_count_size_emptiness_and_duplicates() {
    let mut definition = valid_definition();
    definition.objects = (0..MAX_OBJECTS).map(|i| format!("t{i}")).collect();
    definition.validate().expect("256 distinct objects fit");
    definition.objects = (0..=MAX_OBJECTS).map(|i| format!("t{i}")).collect();
    assert_eq!(
        definition.validate(),
        Err(InvestigationError::TooManyObjects(257))
    );
    definition.objects = vec!["t".repeat(MAX_OBJECT_BYTES)];
    definition.validate().expect("a 256-byte object name fits");
    definition.objects = vec!["t".repeat(MAX_OBJECT_BYTES + 1)];
    assert_eq!(
        definition.validate(),
        Err(InvestigationError::InvalidObject)
    );
    definition.objects = vec!["a".to_string(), "a".to_string()];
    assert_eq!(
        definition.validate(),
        Err(InvestigationError::DuplicateObject)
    );
    definition.objects = vec![String::new()];
    assert_eq!(
        definition.validate(),
        Err(InvestigationError::InvalidObject)
    );
    definition.objects = vec!["a\nb".to_string()];
    assert_eq!(
        definition.validate(),
        Err(InvestigationError::ControlCharacter)
    );
}

#[test]
fn schema_fingerprint_bounds() {
    let mut definition = valid_definition();
    definition.schema_fingerprint = Some("f".repeat(MAX_FINGERPRINT_BYTES));
    definition.validate().expect("128 bytes of fingerprint fit");
    definition.schema_fingerprint = Some("f".repeat(MAX_FINGERPRINT_BYTES + 1));
    assert_eq!(
        definition.validate(),
        Err(InvestigationError::InvalidFingerprint)
    );
    definition.schema_fingerprint = Some(String::new());
    assert_eq!(
        definition.validate(),
        Err(InvestigationError::InvalidFingerprint)
    );
    definition.schema_fingerprint = Some("a\u{0}b".to_string());
    assert_eq!(
        definition.validate(),
        Err(InvestigationError::InvalidFingerprint)
    );
}

#[test]
fn revision_and_timestamp_rules() {
    let mut definition = valid_definition();
    definition.revision = 0;
    assert_eq!(
        definition.validate(),
        Err(InvestigationError::InvalidRevision)
    );
    definition.revision = 1;
    definition.updated_unix_ms = definition.created_unix_ms - 1;
    assert_eq!(
        definition.validate(),
        Err(InvestigationError::UpdatedBeforeCreated)
    );
}

#[test]
fn validate_rejects_wrong_format_and_version_in_code() {
    let mut definition = valid_definition();
    definition.format = "not.saya".to_string();
    assert_eq!(
        definition.validate(),
        Err(InvestigationError::NotAnInvestigation)
    );
    definition.format = INVESTIGATION_FORMAT.to_string();
    definition.version = 2;
    assert_eq!(
        definition.validate(),
        Err(InvestigationError::UnsupportedVersion(2))
    );
}

#[test]
fn new_revision_bumps_revision_and_keeps_identity() {
    let definition = valid_definition();
    let edited = definition.new_revision(
        "select 2",
        "Revenue, revised",
        Some("updated".to_string()),
        definition.updated_unix_ms + 1_000,
    );
    assert_eq!(edited.revision, definition.revision + 1);
    assert_eq!(edited.id, definition.id);
    assert_eq!(edited.created_unix_ms, definition.created_unix_ms);
    assert_eq!(edited.updated_unix_ms, definition.updated_unix_ms + 1_000);
    assert_eq!(edited.sql, "select 2");
    assert_eq!(edited.name, "Revenue, revised");
    assert_eq!(edited.description.as_deref(), Some("updated"));
    assert_eq!(edited.dialect, definition.dialect);
    assert_eq!(edited.connection, definition.connection);
    assert_eq!(edited.objects, definition.objects);
    assert_eq!(edited.format, definition.format);
    assert_eq!(edited.version, definition.version);
    edited
        .validate()
        .expect("an edit of a valid definition validates");
}

#[test]
fn to_json_pretty_enforces_the_document_cap_on_output() {
    let mut definition = valid_definition();
    definition.sql = "s".repeat(MAX_SQL_BYTES);
    definition.description = Some("x".repeat(MAX_DESCRIPTION_BYTES));
    definition.objects = (0..MAX_OBJECTS)
        .map(|i| format!("{i:0>3}{}", "x".repeat(MAX_OBJECT_BYTES - 3)))
        .collect();
    definition
        .validate()
        .expect("every individual bound holds, yet the whole document cannot fit");
    assert!(matches!(
        definition.to_json_pretty(),
        Err(InvestigationError::Oversize(len)) if len > MAX_DEFINITION_BYTES
    ));
}
