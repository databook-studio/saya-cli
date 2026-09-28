//! Tests for the portable context document contract.

use super::*;
use crate::{
    Cardinality, ClaimPayload, ColumnRole, DatabaseObjectKind, DatabaseObjectRef, MAX_NAME_CHARS,
    MAX_TEXT_CHARS, ProfileIdentity,
};

const EXPORTED: i64 = 1_760_000_000_000;

const VERSION_TWO_JSON: &[u8] =
    br#"{"format":"saya.context","version":2,"exported_unix_ms":0,"items":[]}"#;
const VERSION_ZERO_JSON: &[u8] =
    br#"{"format":"saya.context","version":0,"exported_unix_ms":0,"items":[]}"#;
const WRONG_FORMAT_JSON: &[u8] = br#"{"format":"saya.query","version":1}"#;
const MISSING_VERSION_JSON: &[u8] = br#"{"format":"saya.context","exported_unix_ms":0,"items":[]}"#;
const MISSING_ITEMS_JSON: &[u8] = br#"{"format":"saya.context","version":1,"exported_unix_ms":0}"#;
const MISSING_FORMAT_JSON: &[u8] = br#"{"version":1,"exported_unix_ms":0,"items":[]}"#;
const UNKNOWN_ITEM_FIELD_JSON: &[u8] = br#"{"format":"saya.context","version":1,"exported_unix_ms":0,"items":[{"object":{"name":"orders","kind":"table"},"payload":{"kind":"table_alias","alias":"o"},"note":"x"}]}"#;
const UNKNOWN_OBJECT_FIELD_JSON: &[u8] = br#"{"format":"saya.context","version":1,"exported_unix_ms":0,"items":[{"object":{"name":"orders","kind":"table","profile":"p-x"},"payload":{"kind":"table_alias","alias":"o"}}]}"#;
const BAD_PAYLOAD_KIND_JSON: &[u8] = br#"{"format":"saya.context","version":1,"exported_unix_ms":0,"items":[{"object":{"name":"orders","kind":"table"},"payload":{"kind":"nonsense","text":"x"}}]}"#;
const CONTROL_CHAR_TEXT_JSON: &[u8] = br#"{"format":"saya.context","version":1,"exported_unix_ms":0,"items":[{"object":{"name":"orders","kind":"table"},"payload":{"kind":"table_description","text":"bad\u0007char"}}]}"#;

fn object(name: &str) -> PortableObject {
    PortableObject {
        catalog: Some("analytics".to_string()),
        schema: Some("core".to_string()),
        name: name.to_string(),
        kind: DatabaseObjectKind::Table,
    }
}

fn item(name: &str, payload: ClaimPayload) -> ContextItem {
    ContextItem {
        object: object(name),
        payload,
        origin_note: None,
    }
}

fn document(items: Vec<ContextItem>) -> ContextDocumentV1 {
    ContextDocumentV1 {
        format: CONTEXT_FORMAT.to_string(),
        version: CONTEXT_FORMAT_VERSION,
        exported_unix_ms: EXPORTED,
        items,
    }
}

fn validate_item(item: ContextItem) -> Result<(), ContextError> {
    document(vec![item]).validate()
}

/// One item of every payload kind the document carries.
fn representative_document() -> ContextDocumentV1 {
    document(vec![
        item(
            "orders",
            ClaimPayload::table_description("One row per confirmed order.").unwrap(),
        ),
        item(
            "orders",
            ClaimPayload::table_grain("one row per order", Some("shipments split a row per line"))
                .unwrap(),
        ),
        item("orders", ClaimPayload::table_alias("o").unwrap()),
        item(
            "orders",
            ClaimPayload::table_user_note("Refunds stay outside revenue.").unwrap(),
        ),
        item(
            "orders",
            ClaimPayload::join_rule(
                "customers",
                vec!["customer_id".to_string()],
                vec!["id".to_string()],
                "orders.customer_id = customers.id",
                None,
            )
            .unwrap(),
        ),
        item(
            "orders",
            ClaimPayload::metric_definition(
                "net_revenue",
                "sum(amount) where status = 'paid'",
                vec!["amount".to_string()],
                None,
            )
            .unwrap(),
        ),
        item(
            "amount",
            ClaimPayload::column_role("amount", ColumnRole::Measure, Some("tax excluded")).unwrap(),
        ),
        item(
            "orders",
            ClaimPayload::default_time_column("created_at", None).unwrap(),
        ),
        item(
            "created_at",
            ClaimPayload::column_description("created_at", "when the order was placed").unwrap(),
        ),
    ])
}

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
        payload: ClaimPayload::table_alias("vo").unwrap(),
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
    let payload = ClaimPayload::TableDescription {
        text: "x".repeat(MAX_ITEM_BYTES),
    };
    let doc = document(vec![item("orders", payload)]);
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
                ClaimPayload::table_alias(format!("a{i}")).unwrap(),
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
                ClaimPayload::table_alias(format!("a{i}")).unwrap(),
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
        r#"{{"format":"saya.context","version":1,"exported_unix_ms":0,"items":[{{"object":{{"name":"orders","kind":"table"}},"payload":{{"kind":"table_description","text":"{long}"}}}}]}}"#
    );
    assert_eq!(
        ContextDocumentV1::from_json_bytes(oversize_text.as_bytes()),
        Err(ContextError::Malformed)
    );
    assert_eq!(
        ContextDocumentV1::from_json_bytes(CONTROL_CHAR_TEXT_JSON),
        Err(ContextError::Malformed)
    );
    assert_eq!(
        ContextDocumentV1::from_json_bytes(BAD_PAYLOAD_KIND_JSON),
        Err(ContextError::Malformed)
    );
}

#[test]
fn relationship_payloads_are_refused() {
    let profile = ProfileIdentity::parse(&format!("p-{}", "a".repeat(64))).unwrap();
    let target = DatabaseObjectRef::new(
        profile,
        "analytics",
        "core",
        "customers",
        DatabaseObjectKind::Table,
    )
    .unwrap();
    let payload = ClaimPayload::relationship(
        target,
        vec!["customer_id".to_string()],
        vec!["id".to_string()],
        Cardinality::ManyToOne,
    )
    .unwrap();
    let doc = document(vec![item("orders", payload)]);
    assert_eq!(doc.validate(), Err(ContextError::PayloadNotPortable));
    let json = serde_json::to_string(&doc).expect("serializes");
    assert!(
        json.contains("\"profile\""),
        "a relationship serializes its target's profile identity, which is why it is refused"
    );
    assert_eq!(
        ContextDocumentV1::from_json_bytes(json.as_bytes()),
        Err(ContextError::PayloadNotPortable)
    );
}

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
    let mut note = item("orders", ClaimPayload::table_description("ok").unwrap());
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
    let payload = || ClaimPayload::table_alias("o").unwrap();
    let mut it = item("orders", payload());

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
            let mut it = item(&format!("t{i:03}"), payload.clone());
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
