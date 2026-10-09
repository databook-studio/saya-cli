use super::*;

#[test]
fn serialization_leaks_no_identity_state_or_secrets() {
    let json = serde_json::to_string(&representative_document()).expect("serializes");
    for forbidden in [
        "\"profile\"",
        "p-",
        "\"state\"",
        "\"reviewed\"",
        "password",
        "token",
        "credential",
        "created_unix_ms",
        "updated_unix_ms",
    ] {
        assert!(
            !json.contains(forbidden),
            "serialized context must not contain {forbidden:?}"
        );
    }
}

#[test]
fn origin_note_bounds_and_control_characters() {
    let mut note = item("orders", &ClaimPayload::table_description("ok").unwrap());
    note.origin_note = Some("x".repeat(MAX_ORIGIN_NOTE_BYTES));
    validate_item(note.clone()).expect("a 256-byte note fits");
    note.origin_note = Some("x".repeat(MAX_ORIGIN_NOTE_BYTES + 1));
    assert_eq!(
        validate_item(note.clone()),
        Err(ContextError::InvalidOriginNote)
    );
    note.origin_note = Some("note\nline".to_string());
    assert_eq!(
        validate_item(note.clone()),
        Err(ContextError::ControlCharacter)
    );
    note.origin_note = Some("\u{7}".to_string());
    assert_eq!(validate_item(note), Err(ContextError::ControlCharacter));
}

#[test]
fn object_names_are_bounded_like_object_refs() {
    let payload = ClaimPayload::table_alias("o").unwrap();
    let mut it = item("orders", &payload);

    it.object.name = String::new();
    assert_eq!(
        validate_item(it.clone()),
        Err(ContextError::InvalidObjectName)
    );
    it.object.name = "é".repeat(MAX_NAME_CHARS);
    validate_item(it.clone()).expect("128 unicode scalars fit");
    it.object.name = "é".repeat(MAX_NAME_CHARS + 1);
    assert_eq!(
        validate_item(it.clone()),
        Err(ContextError::InvalidObjectName)
    );
    it.object.name = "o\u{7}b".to_string();
    assert_eq!(
        validate_item(it.clone()),
        Err(ContextError::ControlCharacter)
    );

    it.object.name = "orders".to_string();
    it.object.catalog = Some(String::new());
    assert_eq!(
        validate_item(it.clone()),
        Err(ContextError::InvalidObjectName)
    );
    it.object.catalog = Some("analytics".to_string());
    it.object.schema = Some(String::new());
    assert_eq!(
        validate_item(it.clone()),
        Err(ContextError::InvalidObjectName)
    );
    it.object.schema = Some("c\u{7}".to_string());
    assert_eq!(
        validate_item(it.clone()),
        Err(ContextError::ControlCharacter)
    );

    it.object.catalog = None;
    it.object.schema = None;
    validate_item(it).expect("absent catalog and schema are fine");
}

#[test]
fn validate_rejects_wrong_format_and_version_in_code() {
    let mut doc = document(vec![]);
    doc.format = "saya.query".to_string();
    assert_eq!(doc.validate(), Err(ContextError::NotAContextDocument));
    doc.format = CONTEXT_FORMAT.to_string();
    doc.version = 2;
    assert_eq!(doc.validate(), Err(ContextError::UnsupportedVersion(2)));
}

#[test]
fn to_json_pretty_enforces_the_document_cap_on_output() {
    let description = "d".repeat(MAX_TEXT_CHARS);
    let reason = "r".repeat(MAX_TEXT_CHARS);
    let payload = ClaimPayload::table_grain(description, Some(reason.as_str())).unwrap();
    let items: Vec<ContextItem> = (0..MAX_ITEMS)
        .map(|i| {
            let mut it = item(&format!("t{i:03}"), &payload);
            it.origin_note = Some("n".repeat(MAX_ORIGIN_NOTE_BYTES));
            it
        })
        .collect();
    let doc = document(items);
    doc.validate()
        .expect("every item bound holds, yet the whole cannot");
    assert!(matches!(
        doc.to_json_pretty(),
        Err(ContextError::Oversize(size)) if size > MAX_DOCUMENT_BYTES
    ));
}

#[test]
fn probe3() {
    match serde_json::from_slice::<ContextDocumentV1>(CLAIM_WRAPPED_RELATIONSHIP_JSON) {
        Ok(d) => {
            eprintln!("PARSE_OK items={}", d.items.len());
            eprintln!("VALIDATE: {:?}", d.validate());
        }
        Err(e) => eprintln!("PARSE_ERR: {e}"),
    }
}
