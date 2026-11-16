use std::collections::HashSet;

use saya_types::{
    Cardinality, ClaimOrigin, ClaimPayload, ClaimStatus, ContractError, DatabaseObjectKind,
    DatabaseObjectRef, FINGERPRINT_VERSION, MAX_REFERENCED_COLUMNS, ProfileIdentity,
    SchemaFingerprint, SchemaTree, Table,
};

fn valid_hex() -> String {
    "a".repeat(64)
}

fn valid_profile() -> ProfileIdentity {
    ProfileIdentity::parse(&format!("p-{}", valid_hex())).unwrap()
}

fn valid_table() -> Table {
    Table {
        name: "orders".into(),
        columns: vec![saya_types::Column {
            name: "id".into(),
            data_type: "bigint".into(),
            nullable: false,
        }],
        primary_key: vec![],
        foreign_keys: vec![],
    }
}

#[path = "contract_identity/claim_lifecycle.rs"]
mod claim_lifecycle;
#[path = "contract_identity/claim_payload.rs"]
mod claim_payload;
#[path = "contract_identity/fingerprint.rs"]
mod fingerprint;
#[path = "contract_identity/fingerprint_versioning.rs"]
mod fingerprint_versioning;
#[path = "contract_identity/object_ref.rs"]
mod object_ref;
#[path = "contract_identity/profile_identity.rs"]
mod profile_identity;
#[path = "contract_identity/schema_fixtures.rs"]
mod schema_fixtures;
