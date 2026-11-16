use super::*;
use crate::agent::learning::ResolvedProposal;
use crate::agent::learning::turn_record::SuppliedContractDto;
use crate::contracts::{SuppliedClaim, SuppliedContract};
use saya_store::SqliteStateStore;
use saya_types::{
    ClaimId, ClaimOrigin, ClaimPayload, ClaimStatus, ColumnRequirement, DatabaseObjectKind,
    DatabaseObjectRef, KnowledgeSlot, KnowledgeState, ProfileIdentity, SchemaBinding,
    SchemaFingerprint,
};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_root(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "saya-ingest-{label}-{}-{stamp}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

fn test_identity() -> ProfileIdentity {
    ProfileIdentity::parse("p-2222222222222222222222222222222222222222222222222222222222222222")
        .unwrap()
}

fn test_object(name: &str) -> DatabaseObjectRef {
    DatabaseObjectRef::new(
        test_identity(),
        "catalog",
        "public",
        name,
        DatabaseObjectKind::Table,
    )
    .unwrap()
}

mod filtering;
mod persistence;
