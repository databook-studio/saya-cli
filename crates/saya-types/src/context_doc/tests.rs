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
const UNKNOWN_ITEM_FIELD_JSON: &[u8] = br#"{"format":"saya.context","version":1,"exported_unix_ms":0,"items":[{"object":{"name":"orders","kind":"table"},"payload":{"kind":"claim","claim":{"kind":"table_alias","alias":"o"}},"note":"x"}]}"#;
const UNKNOWN_OBJECT_FIELD_JSON: &[u8] = br#"{"format":"saya.context","version":1,"exported_unix_ms":0,"items":[{"object":{"name":"orders","kind":"table","profile":"p-x"},"payload":{"kind":"claim","claim":{"kind":"table_alias","alias":"o"}}}]}"#;
const BAD_PAYLOAD_KIND_JSON: &[u8] = br#"{"format":"saya.context","version":1,"exported_unix_ms":0,"items":[{"object":{"name":"orders","kind":"table"},"payload":{"kind":"nonsense","text":"x"}}]}"#;
const BAD_CLAIM_KIND_JSON: &[u8] = br#"{"format":"saya.context","version":1,"exported_unix_ms":0,"items":[{"object":{"name":"orders","kind":"table"},"payload":{"kind":"claim","claim":{"kind":"nonsense"}}}]}"#;
const CONTROL_CHAR_TEXT_JSON: &[u8] = br#"{"format":"saya.context","version":1,"exported_unix_ms":0,"items":[{"object":{"name":"orders","kind":"table"},"payload":{"kind":"claim","claim":{"kind":"table_description","text":"bad\u0007char"}}}]}"#;
const CLAIM_WRAPPED_JOIN_RULE_JSON: &[u8] = br#"{"format":"saya.context","version":1,"exported_unix_ms":0,"items":[{"object":{"name":"orders","kind":"table"},"payload":{"kind":"claim","claim":{"kind":"join_rule","target":"analytics.core.customers","local_columns":["customer_id"],"target_columns":["id"],"condition":"orders.customer_id = customers.id"}}}]}"#;
const CLAIM_WRAPPED_RELATIONSHIP_JSON: &[u8] = concat!(
    r#"{"format":"saya.context","version":1,"exported_unix_ms":0,"items":[{"object":{"name":"orders","kind":"table"},"payload":{"kind":"claim","claim":{"kind":"relationship","target":{"profile":""#,
    "p-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    r#"","catalog":"analytics","schema":"core","object":"customers","kind":"table"},"local_columns":["customer_id"],"target_columns":["id"],"cardinality":"many_to_one"}}}]}"#,
)
.as_bytes();

fn profile() -> ProfileIdentity {
    ProfileIdentity::parse(&format!("p-{}", "a".repeat(64))).unwrap()
}

fn object(name: &str) -> PortableObject {
    PortableObject {
        catalog: Some("analytics".to_string()),
        schema: Some("core".to_string()),
        name: name.to_string(),
        kind: DatabaseObjectKind::Table,
    }
}

fn item(name: &str, payload: &ClaimPayload) -> ContextItem {
    ContextItem {
        object: object(name),
        payload: PortablePayload::from_claim(payload)
            .expect("every claim kind has a portable form"),
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

/// Resolves a logical object to a ref in the same shape it arrived: absent
/// parts fall back to the fixture's catalog and schema.
fn resolve_with(profile: ProfileIdentity) -> impl Fn(&PortableObject) -> Option<DatabaseObjectRef> {
    move |target: &PortableObject| {
        DatabaseObjectRef::new(
            profile.clone(),
            target
                .catalog
                .clone()
                .unwrap_or_else(|| "analytics".to_string()),
            target.schema.clone().unwrap_or_else(|| "core".to_string()),
            target.name.clone(),
            target.kind,
        )
        .ok()
    }
}

/// A relationship claim whose target carries a (fixture) profile identity:
/// the portable form must strip it.
fn relationship_item() -> ContextItem {
    let target = DatabaseObjectRef::new(
        profile(),
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
    item("orders", &payload)
}

/// One item of every payload kind the document carries.
fn representative_document() -> ContextDocumentV1 {
    document(vec![
        item(
            "orders",
            &ClaimPayload::table_description("One row per confirmed order.").unwrap(),
        ),
        item(
            "orders",
            &ClaimPayload::table_grain("one row per order", Some("shipments split a row per line"))
                .unwrap(),
        ),
        item("orders", &ClaimPayload::table_alias("o").unwrap()),
        item(
            "orders",
            &ClaimPayload::table_user_note("Refunds stay outside revenue.").unwrap(),
        ),
        item(
            "orders",
            &ClaimPayload::join_rule(
                "analytics.core.customers",
                vec!["customer_id".to_string()],
                vec!["id".to_string()],
                "orders.customer_id = customers.id",
                None,
            )
            .unwrap(),
        ),
        item(
            "orders",
            &ClaimPayload::metric_definition(
                "net_revenue",
                "sum(amount) where status = 'paid'",
                vec!["amount".to_string()],
                None,
            )
            .unwrap(),
        ),
        item(
            "amount",
            &ClaimPayload::column_role("amount", ColumnRole::Measure, Some("tax excluded"))
                .unwrap(),
        ),
        item(
            "orders",
            &ClaimPayload::default_time_column("created_at", None).unwrap(),
        ),
        item(
            "created_at",
            &ClaimPayload::column_description("created_at", "when the order was placed").unwrap(),
        ),
        relationship_item(),
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

#[test]
fn relationship_items_travel_without_identity() {
    let doc = document(vec![relationship_item()]);
    doc.validate().expect("a portable relationship validates");
    let json = serde_json::to_string(&doc).expect("serializes");
    for forbidden in ["\"profile\"", "p-"] {
        assert!(
            !json.contains(forbidden),
            "a portable relationship must not contain {forbidden:?}"
        );
    }
    let reparsed =
        ContextDocumentV1::from_json_bytes(json.as_bytes()).expect("portable output reparses");
    assert_eq!(reparsed, doc);

    // The rebuilt claim is the original, resolved against the profile.
    let rebuilt = reparsed.items[0]
        .payload
        .clone()
        .into_claim(resolve_with(profile()))
        .expect("the target resolves");
    let expected = ClaimPayload::relationship(
        DatabaseObjectRef::new(
            profile(),
            "analytics",
            "core",
            "customers",
            DatabaseObjectKind::Table,
        )
        .unwrap(),
        vec!["customer_id".to_string()],
        vec!["id".to_string()],
        Cardinality::ManyToOne,
    )
    .unwrap();
    assert_eq!(rebuilt, expected);
}

#[test]
fn join_rule_items_requalify_their_target() {
    let payload = ClaimPayload::join_rule(
        "analytics.core.customers",
        vec!["customer_id".to_string()],
        vec!["id".to_string()],
        "orders.customer_id = customers.id",
        None,
    )
    .unwrap();
    let portable = PortablePayload::from_claim(&payload).unwrap();
    match &portable {
        PortablePayload::JoinRule { target, .. } => {
            assert_eq!(target.catalog.as_deref(), Some("analytics"));
            assert_eq!(target.schema.as_deref(), Some("core"));
            assert_eq!(target.name, "customers");
        }
        _ => panic!("a join rule becomes the portable variant"),
    }
    let doc = document(vec![ContextItem {
        object: object("orders"),
        payload: portable,
        origin_note: None,
    }]);
    let json = serde_json::to_string(&doc).expect("serializes");
    assert!(
        !json.contains("p-"),
        "a portable join rule carries no identity"
    );
    let reparsed =
        ContextDocumentV1::from_json_bytes(json.as_bytes()).expect("portable output reparses");
    let rebuilt = reparsed.items[0]
        .payload
        .clone()
        .into_claim(resolve_with(profile()))
        .expect("the target resolves");
    assert_eq!(
        rebuilt, payload,
        "the resolved ref's qualified name restores the original target"
    );
}

#[test]
fn join_rule_targets_without_three_parts_travel_as_a_bare_name() {
    let payload = ClaimPayload::join_rule(
        "public.customers",
        vec!["customer_id".to_string()],
        vec!["id".to_string()],
        "orders.customer_id = customers.id",
        None,
    )
    .unwrap();
    let portable = PortablePayload::from_claim(&payload).unwrap();
    match &portable {
        PortablePayload::JoinRule { target, .. } => {
            assert_eq!(target.catalog, None);
            assert_eq!(target.schema, None);
            assert_eq!(target.name, "public.customers");
        }
        _ => panic!("a join rule becomes the portable variant"),
    }
    // The resolver remaps the bare name: the rebuilt target is the resolved
    // ref's qualified name in the destination's naming, never the source's.
    let rebuilt = portable.into_claim(resolve_with(profile())).unwrap();
    match rebuilt {
        ClaimPayload::JoinRule { target, .. } => {
            assert_eq!(target, "analytics.core.public.customers");
        }
        _ => panic!("the join rule rebuilds"),
    }
}

#[test]
fn claim_wrapper_refuses_target_bearing_payloads() {
    let relationship = ClaimPayload::relationship(
        DatabaseObjectRef::new(
            profile(),
            "analytics",
            "core",
            "customers",
            DatabaseObjectKind::Table,
        )
        .unwrap(),
        vec!["customer_id".to_string()],
        vec!["id".to_string()],
        Cardinality::ManyToOne,
    )
    .unwrap();
    let wrapped = PortablePayload::Claim {
        claim: relationship.clone(),
    };
    let doc = document(vec![ContextItem {
        object: object("orders"),
        payload: wrapped,
        origin_note: None,
    }]);
    assert_eq!(doc.validate(), Err(ContextError::PayloadNotPortable));
    assert!(matches!(
        doc.to_json_pretty(),
        Err(ContextError::PayloadNotPortable)
    ));
    assert_eq!(
        ContextDocumentV1::from_json_bytes(CLAIM_WRAPPED_RELATIONSHIP_JSON),
        Err(ContextError::PayloadNotPortable)
    );
    assert_eq!(
        ContextDocumentV1::from_json_bytes(CLAIM_WRAPPED_JOIN_RULE_JSON),
        Err(ContextError::PayloadNotPortable)
    );
    // into_claim refuses too, and never consults a resolver for it.
    let rebuilt = PortablePayload::Claim {
        claim: relationship,
    }
    .into_claim(|_| None);
    assert_eq!(rebuilt, Err(ContextError::PayloadNotPortable));
}

#[test]
fn unresolved_targets_are_refused() {
    let rebuilt = document(vec![relationship_item()]).items[0]
        .payload
        .clone()
        .into_claim(|_| None);
    assert_eq!(rebuilt, Err(ContextError::UnresolvedTarget));
    let payload = ClaimPayload::join_rule(
        "analytics.core.customers",
        vec!["customer_id".to_string()],
        vec!["id".to_string()],
        "a = b",
        None,
    )
    .unwrap();
    let rebuilt = PortablePayload::from_claim(&payload)
        .unwrap()
        .into_claim(|_| None);
    assert_eq!(rebuilt, Err(ContextError::UnresolvedTarget));
}

#[test]
fn portable_relationship_validates_its_columns() {
    let target = PortableObject {
        catalog: Some("analytics".to_string()),
        schema: Some("core".to_string()),
        name: "customers".to_string(),
        kind: DatabaseObjectKind::Table,
    };
    let payload = |local: Vec<String>, target_columns: Vec<String>| PortablePayload::Relationship {
        target: target.clone(),
        local_columns: local,
        target_columns,
        cardinality: Cardinality::ManyToOne,
    };
    assert_eq!(
        payload(vec!["a".to_string()], vec![]).validate(),
        Err(ContextError::InvalidColumns)
    );
    assert_eq!(
        payload(vec![], vec![]).validate(),
        Err(ContextError::InvalidColumns),
        "relationship columns must be non-empty"
    );
    let many: Vec<String> = (0..33).map(|i| format!("c{i}")).collect();
    assert_eq!(
        payload(many.clone(), many).validate(),
        Err(ContextError::InvalidColumns)
    );
    assert_eq!(
        payload(vec!["a\nb".to_string()], vec!["a".to_string()]).validate(),
        Err(ContextError::ControlCharacter)
    );
    let pair = payload(vec!["customer_id".to_string()], vec!["id".to_string()]);
    pair.validate().expect("a valid column pair passes");
}

#[test]
fn portable_join_rule_validates_its_columns_and_text() {
    let target = PortableObject {
        catalog: None,
        schema: None,
        name: "customers".to_string(),
        kind: DatabaseObjectKind::Table,
    };
    let rule = |condition: &str, reason: Option<&str>| PortablePayload::JoinRule {
        target: target.clone(),
        local_columns: vec![],
        target_columns: vec![],
        condition: condition.to_string(),
        reason: reason.map(str::to_string),
    };
    rule("a = b", None)
        .validate()
        .expect("a join rule may carry no columns at all");
    assert_eq!(
        PortablePayload::JoinRule {
            target: target.clone(),
            local_columns: vec!["a".to_string()],
            target_columns: vec![],
            condition: "a = b".to_string(),
            reason: None,
        }
        .validate(),
        Err(ContextError::InvalidColumns),
        "a half-empty column pair does not pair"
    );
    assert_eq!(rule("", None).validate(), Err(ContextError::InvalidText));
    assert_eq!(
        rule("a\u{7}b", None).validate(),
        Err(ContextError::ControlCharacter)
    );
    assert_eq!(
        rule(&"x".repeat(MAX_TEXT_CHARS + 1), None).validate(),
        Err(ContextError::InvalidText)
    );
    rule(&"x".repeat(MAX_TEXT_CHARS), None)
        .validate()
        .expect("1024 characters fit");
    assert_eq!(
        rule("a = b", Some("r\n")).validate(),
        Err(ContextError::ControlCharacter)
    );
    assert_eq!(
        rule("a = b", Some(&"x".repeat(MAX_TEXT_CHARS + 1))).validate(),
        Err(ContextError::InvalidText)
    );
    // A whitespace-only reason survives the gate; the rebuild trims it away.
    rule("a = b", Some("   "))
        .validate()
        .expect("whitespace-only reason is text-shaped");
    let rebuilt = rule("a = b", Some("   "))
        .into_claim(resolve_with(profile()))
        .expect("the rebuild resolves");
    assert!(
        matches!(rebuilt, ClaimPayload::JoinRule { reason: None, .. }),
        "a whitespace-only reason collapses to None on rebuild"
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
