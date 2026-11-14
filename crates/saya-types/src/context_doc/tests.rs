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

#[path = "tests/document_metadata.rs"]
mod document_metadata;
#[path = "tests/document_wire.rs"]
mod document_wire;
#[path = "tests/portable_conversions.rs"]
mod portable_conversions;
#[path = "tests/portable_validation.rs"]
mod portable_validation;
