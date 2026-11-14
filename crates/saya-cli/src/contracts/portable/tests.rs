//! Unit tests for the portable mapping: resolution, ambiguity, the
//! relationship→join-rule filing, and the export document shape. The full
//! command flows live in `tests/contracts_portable.rs`.

use super::map_items;
use crate::contracts::portable::{PortableError, export, import_document, read_context};
use saya_store::{KnowledgeItemRequest, KnowledgeItemStore, SqliteStateStore};
use saya_types::{
    CONTEXT_FORMAT, CONTEXT_FORMAT_VERSION, ClaimOrigin, ClaimPayload, Column, ContextError,
    ContextItem, Database, DatabaseObjectKind, DatabaseObjectRef, KnowledgeSlot, KnowledgeState,
    MAX_DOCUMENT_BYTES, PortableObject, PortablePayload, ProfileIdentity, Schema, SchemaBinding,
    SchemaFingerprint, SchemaTree, Table,
};

fn identity() -> ProfileIdentity {
    ProfileIdentity::parse(&format!("p-{}", "a".repeat(64))).unwrap()
}

/// `(schema, table name, columns)` triples of one schema under one catalog.
type Tables<'a> = &'a [(&'a str, &'a str, &'a [(&'a str, &'a str)])];

fn tree(tables: Tables<'_>) -> SchemaTree {
    SchemaTree {
        databases: vec![Database {
            name: "analytics".into(),
            schemas: vec![Schema {
                name: "public".into(),
                tables: tables
                    .iter()
                    .map(|(_schema, name, columns)| Table {
                        name: (*name).into(),
                        columns: columns
                            .iter()
                            .map(|(name, data_type)| Column {
                                name: (*name).into(),
                                data_type: (*data_type).into(),
                                nullable: true,
                            })
                            .collect(),
                        primary_key: vec![],
                        foreign_keys: vec![],
                    })
                    .collect(),
            }],
        }],
    }
}

fn item(name: &str, payload: &ClaimPayload) -> ContextItem {
    ContextItem {
        object: PortableObject {
            catalog: Some("analytics".into()),
            schema: Some("public".into()),
            name: name.into(),
            kind: DatabaseObjectKind::Table,
        },
        payload: PortablePayload::from_claim(payload).unwrap(),
        origin_note: Some("dbt 1.8.6 model.jaffle.orders".into()),
    }
}

include!("tests/resolution.rs");

include!("tests/redaction_and_round_trip.rs");
include!("tests/batch_refusals.rs");
include!("tests/batch_success.rs");
