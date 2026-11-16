use super::*;
use crate::contract::claim::{CLAIM_PAYLOAD_VERSION, MAX_TEXT_CHARS};
use crate::contract::claim_enums::{Cardinality, ColumnRole};
use crate::{DatabaseObjectKind, ProfileIdentity};

fn make_profile() -> ProfileIdentity {
    ProfileIdentity::parse(&format!("p-{}", "a".repeat(64))).unwrap()
}

fn make_target() -> DatabaseObjectRef {
    let profile = make_profile();
    DatabaseObjectRef::new(profile, "c", "s", "t", DatabaseObjectKind::Table).unwrap()
}

#[path = "claim_payload_tests/blanking.rs"]
mod blanking;
#[path = "claim_payload_tests/metrics.rs"]
mod metrics;
#[path = "claim_payload_tests/payload_inspection.rs"]
mod payload_inspection;
#[path = "claim_payload_tests/relationships.rs"]
mod relationships;
#[path = "claim_payload_tests/snapshots.rs"]
mod snapshots;
#[path = "claim_payload_tests/text_validation.rs"]
mod text_validation;
#[path = "claim_payload_tests/version.rs"]
mod version;
