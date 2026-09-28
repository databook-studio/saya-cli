//! Tests for the saved-investigation contract.

use super::*;
use proptest::prelude::*;

const CREATED: i64 = 1_700_000_000_000;

const VERSION_TWO_JSON: &[u8] = concat!(
    r#"{"format":"saya.investigation","version":2,"id":"rev-12345678","revision":1,"#,
    r#""name":"n","sql":"select 1","dialect":"postgresql","connection":"c","#,
    r#""objects":[],"schema_fingerprint":null,"created_unix_ms":1,"updated_unix_ms":1}"#,
)
.as_bytes();

const WRONG_FORMAT_JSON: &[u8] = br#"{"format":"saya.query","version":1}"#;

const MISSING_VERSION_JSON: &[u8] = br#"{"format":"saya.investigation","id":"rev-12345678"}"#;

const INVALID_ID_JSON: &[u8] = concat!(
    r#"{"format":"saya.investigation","version":1,"id":"Bad","revision":1,"name":"n","#,
    r#""sql":"select 1","dialect":"postgresql","connection":"c","objects":[],"#,
    r#""schema_fingerprint":null,"created_unix_ms":1,"updated_unix_ms":1}"#,
)
.as_bytes();

fn valid_definition() -> InvestigationDefinitionV1 {
    InvestigationDefinitionV1 {
        format: INVESTIGATION_FORMAT.to_string(),
        version: INVESTIGATION_FORMAT_VERSION,
        id: InvestigationId::derive("revenue", "select 1", CREATED),
        revision: 1,
        name: "Revenue".to_string(),
        description: None,
        sql: "select 1".to_string(),
        dialect: SqlDialect::Postgres,
        connection: "prod".to_string(),
        objects: vec!["public.orders".to_string()],
        schema_fingerprint: None,
        created_unix_ms: CREATED,
        updated_unix_ms: CREATED,
    }
}

fn definition_json(definition: &InvestigationDefinitionV1) -> String {
    serde_json::to_string_pretty(definition).expect("a valid definition serializes")
}

#[test]
fn definition_rejects_oversize_and_unknown_version() {
    let oversized = vec![b' '; MAX_DEFINITION_BYTES + 1];
    assert_eq!(
        InvestigationDefinitionV1::from_json_bytes(&oversized),
        Err(InvestigationError::Oversize(MAX_DEFINITION_BYTES + 1))
    );

    assert_eq!(
        InvestigationDefinitionV1::from_json_bytes(VERSION_TWO_JSON),
        Err(InvestigationError::UnsupportedVersion(2))
    );

    let definition = valid_definition();
    let reparsed =
        InvestigationDefinitionV1::from_json_bytes(definition_json(&definition).as_bytes())
            .expect("a version-1 definition reparses");
    assert_eq!(reparsed, definition);
    let pretty = definition
        .to_json_pretty()
        .expect("a valid definition serializes");
    assert_eq!(
        InvestigationDefinitionV1::from_json_bytes(pretty.as_bytes())
            .expect("pretty output reparses"),
        definition
    );
}

#[test]
fn definition_rejects_unknown_field() {
    let mut json = definition_json(&valid_definition());
    assert!(json.ends_with('}'));
    json.pop();
    json.push_str(", \"sneaky_field\": 1 }");
    assert!(matches!(
        InvestigationDefinitionV1::from_json_bytes(json.as_bytes()),
        Err(InvestigationError::Malformed)
    ));
}

#[test]
fn definition_rejects_wrong_format() {
    assert_eq!(
        InvestigationDefinitionV1::from_json_bytes(WRONG_FORMAT_JSON),
        Err(InvestigationError::NotAnInvestigation)
    );
}

#[test]
fn definition_rejects_missing_version_field() {
    assert!(matches!(
        InvestigationDefinitionV1::from_json_bytes(MISSING_VERSION_JSON),
        Err(InvestigationError::Malformed)
    ));
}

#[test]
fn definition_rejects_an_invalid_id_through_serde() {
    assert!(matches!(
        InvestigationDefinitionV1::from_json_bytes(INVALID_ID_JSON),
        Err(InvestigationError::Malformed)
    ));
}

#[test]
fn name_bounds_are_unicode_scalars_not_bytes() {
    let mut definition = valid_definition();
    definition.name = "é".repeat(MAX_NAME_CHARS);
    definition
        .validate()
        .expect("80 unicode scalars fit even in 160 bytes");
    definition.name = "é".repeat(MAX_NAME_CHARS + 1);
    assert_eq!(definition.validate(), Err(InvestigationError::InvalidName));
}

#[test]
fn name_rejects_trimmed_empty_and_control_characters() {
    let mut definition = valid_definition();
    definition.name = "   ".to_string();
    assert_eq!(definition.validate(), Err(InvestigationError::InvalidName));
    definition.name = "a\nb".to_string();
    assert_eq!(
        definition.validate(),
        Err(InvestigationError::ControlCharacter)
    );
    definition.name = "  Revenue  ".to_string();
    definition
        .validate()
        .expect("surrounding whitespace is tolerated; trimming is the caller's concern");
}

#[test]
fn description_bounds_allow_only_newline_control_characters() {
    let mut definition = valid_definition();
    definition.description = Some("x".repeat(MAX_DESCRIPTION_BYTES));
    definition.validate().expect("2048 bytes fit");
    definition.description = Some("x".repeat(MAX_DESCRIPTION_BYTES + 1));
    assert_eq!(
        definition.validate(),
        Err(InvestigationError::DescriptionTooLong)
    );
    definition.description = Some("line one\nline two".to_string());
    definition
        .validate()
        .expect("newlines are allowed in a description");
    definition.description = Some("tab\tstop".to_string());
    assert_eq!(
        definition.validate(),
        Err(InvestigationError::ControlCharacter)
    );
}

#[test]
fn sql_bounds_and_trimmed_emptiness() {
    let mut definition = valid_definition();
    definition.sql = "s".repeat(MAX_SQL_BYTES);
    definition.validate().expect("64 KiB of sql fits");
    definition.sql = "s".repeat(MAX_SQL_BYTES + 1);
    assert_eq!(definition.validate(), Err(InvestigationError::InvalidSql));
    definition.sql = " \n ".to_string();
    assert_eq!(definition.validate(), Err(InvestigationError::InvalidSql));
}

#[test]
fn sql_rejects_terminal_control_characters() {
    let mut definition = valid_definition();
    for sql in [
        // ESC opening an OSC sequence inside a comment.
        "-- \u{1b}]0;pwned\u{7}\nselect 1",
        // ESC opening a CSI sequence.
        "-- \u{1b}[31mred\nselect 1",
        // BEL on its own.
        "select 1 -- \u{7}",
        // NUL.
        "select\u{0}1",
        // U+009B, the single-character C1 CSI.
        "select 1 -- \u{9b}31m",
        // DEL.
        "select 1 -- \u{7f}",
    ] {
        definition.sql = sql.to_string();
        assert_eq!(
            definition.validate(),
            Err(InvestigationError::ControlCharacter),
            "{sql:?} must be refused"
        );
    }
}

#[test]
fn sql_keeps_newlines_tabs_and_crlf() {
    let mut definition = valid_definition();
    definition.sql = "select 1,\r\n\t2\r\nfrom t\n".to_string();
    definition
        .validate()
        .expect("newlines, tabs, and CRLF are still allowed");
}

#[test]
fn connection_bounds_and_charset() {
    let mut definition = valid_definition();
    definition.connection = "a".repeat(MAX_CONNECTION_CHARS);
    definition
        .validate()
        .expect("64 characters of connection alias fit");
    definition.connection = "a".repeat(MAX_CONNECTION_CHARS + 1);
    assert_eq!(
        definition.validate(),
        Err(InvestigationError::InvalidConnection)
    );
    definition.connection = String::new();
    assert_eq!(
        definition.validate(),
        Err(InvestigationError::InvalidConnection)
    );
    definition.connection = "prod/db".to_string();
    assert_eq!(
        definition.validate(),
        Err(InvestigationError::InvalidConnection)
    );
    definition.connection = "prod.db_1-x".to_string();
    definition
        .validate()
        .expect("dots, underscores, and dashes are allowed");
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
fn investigation_id_derives_deterministic_valid_ids() {
    for name in ["  Revenue — Q3 / EU!! ", "", "---"] {
        let first = InvestigationId::derive(name, "select 1", CREATED);
        let second = InvestigationId::derive(name, "select 1", CREATED);
        assert_eq!(first, second, "derive is deterministic for {name:?}");
        InvestigationId::parse(first.as_str())
            .unwrap_or_else(|_| panic!("derived id {:?} must always be valid", first.as_str()));
    }
    assert!(
        InvestigationId::derive("  Revenue — Q3 / EU!! ", "select 1", CREATED)
            .as_str()
            .starts_with("revenue-q3-eu-")
    );
    assert!(
        InvestigationId::derive("", "select 1", CREATED)
            .as_str()
            .starts_with("investigation-")
    );
    assert!(
        InvestigationId::derive("---", "select 1", CREATED)
            .as_str()
            .starts_with("investigation-")
    );

    let long = InvestigationId::derive(&"w".repeat(100), "select 1", CREATED);
    assert_eq!(long.as_str().len(), 48 + 1 + 8);
    InvestigationId::parse(long.as_str()).expect("a capped slug still yields a valid id");

    assert_ne!(
        InvestigationId::derive("a", "select 1", CREATED),
        InvestigationId::derive("a", "select 2", CREATED)
    );
    assert_ne!(
        InvestigationId::derive("a", "select 1", CREATED),
        InvestigationId::derive("a", "select 1", CREATED + 1)
    );
}

#[test]
fn investigation_id_parse_rejects_invalid_shapes() {
    for bad in ["A", "-a", "a-", "a--b", "a/b", ".."] {
        assert_eq!(
            InvestigationId::parse(bad),
            Err(InvestigationError::InvalidId),
            "{bad:?} must be rejected"
        );
    }
    assert_eq!(
        InvestigationId::parse(&"a".repeat(MAX_ID_CHARS + 1)),
        Err(InvestigationError::InvalidId)
    );
    assert_eq!(
        InvestigationId::parse(""),
        Err(InvestigationError::InvalidId)
    );
    assert!(InvestigationId::parse("a").is_ok());
    InvestigationId::parse(&"a".repeat(MAX_ID_CHARS)).expect("64 characters fit");
}

#[test]
fn investigation_id_round_trips_through_serde() {
    let id = InvestigationId::parse("revenue-12345678").expect("a well-shaped id parses");
    assert_eq!(
        serde_json::to_string(&id).expect("ids serialize"),
        "\"revenue-12345678\""
    );
    assert_eq!(
        serde_json::from_str::<InvestigationId>("\"revenue-12345678\"").expect("ids deserialize"),
        id
    );
    assert!(serde_json::from_str::<InvestigationId>("\"Not-Valid\"").is_err());
}

#[test]
fn serialization_carries_no_secret_shaped_keys() {
    let json = valid_definition()
        .to_json_pretty()
        .expect("a valid definition serializes");
    for forbidden in ["password", "token", "rows", "result", "grant", "identity"] {
        assert!(
            !json.contains(forbidden),
            "serialized definitions must not contain {forbidden:?}"
        );
    }
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

proptest! {
    /// Any name, however hostile, derives an id its own `parse` accepts, and
    /// derivation never wavers for the same inputs.
    #[test]
    fn derived_id_is_always_valid_and_deterministic(
        name in "(?s).{0,120}",
        sql in "(?s).{0,120}",
        created_unix_ms in any::<i64>(),
    ) {
        let first = InvestigationId::derive(&name, &sql, created_unix_ms);
        let second = InvestigationId::derive(&name, &sql, created_unix_ms);
        prop_assert!(InvestigationId::parse(first.as_str()).is_ok());
        prop_assert_eq!(first, second);
    }
}
