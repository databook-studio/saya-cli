use super::*;

#[test]
fn round_trip_preserves_the_document() {
    let doc = representative_document();
    let pretty = doc.to_json_pretty().expect("a valid document serializes");
    assert_eq!(
        ContextDocumentV1::from_json_bytes(pretty.as_bytes()).expect("pretty output reparses"),
        doc
    );
    let compact = serde_json::to_vec(&doc).expect("a valid document serializes");
    assert_eq!(
        ContextDocumentV1::from_json_bytes(&compact).expect("compact output reparses"),
        doc
    );
}

#[test]
fn optional_fields_round_trip() {
    let mut doc = representative_document();
    doc.items.push(ContextItem {
        object: PortableObject {
            catalog: None,
            schema: None,
            name: "v_orders".to_string(),
            kind: DatabaseObjectKind::View,
        },
        payload: PortablePayload::from_claim(&ClaimPayload::table_alias("vo").unwrap()).unwrap(),
        origin_note: Some("from the analytics team".to_string()),
    });
    let json = doc.to_json_pretty().expect("serializes");
    assert_eq!(
        ContextDocumentV1::from_json_bytes(json.as_bytes()).unwrap(),
        doc
    );
    assert!(
        json.contains("\"origin_note\""),
        "a present note serializes"
    );
    assert!(
        !json.contains("\"catalog\":null"),
        "absent catalog is omitted, never null"
    );
}

#[test]
fn empty_documents_round_trip() {
    let doc = document(vec![]);
    doc.validate().expect("an empty document is valid");
    let json = doc.to_json_pretty().expect("serializes");
    assert_eq!(
        ContextDocumentV1::from_json_bytes(json.as_bytes()).unwrap(),
        doc
    );
}

#[test]
fn oversize_document_is_refused_before_parsing() {
    let oversized = vec![b' '; MAX_DOCUMENT_BYTES + 1];
    assert_eq!(
        ContextDocumentV1::from_json_bytes(&oversized),
        Err(ContextError::Oversize(MAX_DOCUMENT_BYTES + 1))
    );
    let at_cap = vec![b' '; MAX_DOCUMENT_BYTES];
    assert_eq!(
        ContextDocumentV1::from_json_bytes(&at_cap),
        Err(ContextError::Malformed),
        "at the cap the probe runs and fails; the cap itself passes"
    );
}

#[test]
fn oversize_item_is_refused() {
    // A payload's own validator caps its text at MAX_TEXT_CHARS, so an item
    // over the 4 KiB gate can only be assembled in code, inside this crate —
    // and validate must still catch it, however it arrived.
    let payload = PortablePayload::Claim {
        claim: ClaimPayload::TableDescription {
            text: "x".repeat(MAX_ITEM_BYTES),
        },
    };
    let doc = document(vec![ContextItem {
        object: object("orders"),
        payload,
        origin_note: None,
    }]);
    assert!(matches!(
        doc.validate(),
        Err(ContextError::ItemOversize(size)) if size > MAX_ITEM_BYTES
    ));
    assert!(matches!(
        doc.to_json_pretty(),
        Err(ContextError::ItemOversize(_))
    ));
}

#[test]
fn too_many_items_are_refused() {
    let over: Vec<ContextItem> = (0..=MAX_ITEMS)
        .map(|i| {
            item(
                &format!("t{i}"),
                &ClaimPayload::table_alias(format!("a{i}")).unwrap(),
            )
        })
        .collect();
    assert_eq!(
        document(over).validate(),
        Err(ContextError::TooManyItems(MAX_ITEMS + 1))
    );
    let exact: Vec<ContextItem> = (0..MAX_ITEMS)
        .map(|i| {
            item(
                &format!("t{i}"),
                &ClaimPayload::table_alias(format!("a{i}")).unwrap(),
            )
        })
        .collect();
    document(exact).validate().expect("500 items fit");
}

#[test]
fn unknown_version_is_named() {
    assert_eq!(
        ContextDocumentV1::from_json_bytes(VERSION_TWO_JSON),
        Err(ContextError::UnsupportedVersion(2))
    );
    assert_eq!(
        ContextDocumentV1::from_json_bytes(VERSION_ZERO_JSON),
        Err(ContextError::UnsupportedVersion(0))
    );
}

#[test]
fn foreign_or_truncated_documents_are_refused() {
    assert_eq!(
        ContextDocumentV1::from_json_bytes(WRONG_FORMAT_JSON),
        Err(ContextError::NotAContextDocument)
    );
    assert_eq!(
        ContextDocumentV1::from_json_bytes(MISSING_FORMAT_JSON),
        Err(ContextError::NotAContextDocument)
    );
    assert_eq!(
        ContextDocumentV1::from_json_bytes(MISSING_VERSION_JSON),
        Err(ContextError::Malformed)
    );
    assert_eq!(
        ContextDocumentV1::from_json_bytes(MISSING_ITEMS_JSON),
        Err(ContextError::Malformed),
        "items is required; a document without it is truncated"
    );
    assert_eq!(
        ContextDocumentV1::from_json_bytes(b"not json"),
        Err(ContextError::Malformed)
    );
}

#[test]
fn unknown_fields_are_refused_at_every_level() {
    let mut top = representative_document()
        .to_json_pretty()
        .expect("serializes");
    top.pop();
    top.push_str(", \"sneaky\": 1}");
    assert_eq!(
        ContextDocumentV1::from_json_bytes(top.as_bytes()),
        Err(ContextError::Malformed)
    );
    for json in [UNKNOWN_ITEM_FIELD_JSON, UNKNOWN_OBJECT_FIELD_JSON] {
        assert_eq!(
            ContextDocumentV1::from_json_bytes(json),
            Err(ContextError::Malformed),
            "{json:?} must be refused"
        );
    }
}

#[test]
fn payloads_that_fail_their_validator_are_rejected() {
    let long = "x".repeat(MAX_TEXT_CHARS + 1);
    let oversize_text = format!(
        r#"{{"format":"saya.context","version":1,"exported_unix_ms":0,"items":[{{"object":{{"name":"orders","kind":"table"}},"payload":{{"kind":"claim","claim":{{"kind":"table_description","text":"{long}"}}}}}}]}}"#
    );
    assert_eq!(
        ContextDocumentV1::from_json_bytes(oversize_text.as_bytes()),
        Err(ContextError::Malformed)
    );
    assert_eq!(
        ContextDocumentV1::from_json_bytes(CONTROL_CHAR_TEXT_JSON),
        Err(ContextError::Malformed)
    );
    for json in [BAD_PAYLOAD_KIND_JSON, BAD_CLAIM_KIND_JSON] {
        assert_eq!(
            ContextDocumentV1::from_json_bytes(json),
            Err(ContextError::Malformed),
            "{json:?} must be refused"
        );
    }
}
