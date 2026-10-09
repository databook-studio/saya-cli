//! Tests for the saved-investigation document repository (D2): bounded,
//! atomic, revision-checked JSON documents at `<root>/<id>.json` plus the
//! per-machine local binding at `<root>/local/<id>.json`.

use crate::investigations::{
    InvestigationListIssue, InvestigationRepository, InvestigationSummary, LocalBinding,
};
use crate::{StoreError, replace::FailingReplacer};
use saya_types::{
    MAX_SQL_BYTES, SqlDialect,
    investigation::{
        INVESTIGATION_FORMAT, INVESTIGATION_FORMAT_VERSION, InvestigationDefinitionV1,
        InvestigationId,
    },
};
use std::path::PathBuf;

fn temp_root(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "saya-investigation-docs-{label}-{}",
        std::process::id()
    ))
}

fn id(name: &str) -> InvestigationId {
    InvestigationId::parse(name).unwrap()
}

fn definition(name_id: &str, revision: u32) -> InvestigationDefinitionV1 {
    InvestigationDefinitionV1 {
        format: INVESTIGATION_FORMAT.to_owned(),
        version: INVESTIGATION_FORMAT_VERSION,
        id: id(name_id),
        revision,
        name: "Demo investigation".to_owned(),
        description: None,
        sql: format!("select {revision}"),
        parameters: Vec::new(),
        dialect: SqlDialect::Sqlite,
        connection: "warehouse".to_owned(),
        objects: Vec::new(),
        schema_fingerprint: None,
        created_unix_ms: 1_000,
        updated_unix_ms: 1_000,
    }
}

fn document_bytes(name_id: &str, revision: u32) -> String {
    definition(name_id, revision).to_json_pretty().unwrap()
}

fn binding(name_id: &str) -> LocalBinding {
    LocalBinding {
        version: LocalBinding::VERSION,
        id: id(name_id),
        profile: "warehouse".to_owned(),
        profile_identity: "identity-1".to_owned(),
        reviewed_revision: 2,
        reviewed_schema_fingerprint: Some("schema-fp-1".to_owned()),
        reviewed_unix_ms: 2_000,
    }
}

fn summary_ids(page: &crate::investigations::InvestigationPage) -> Vec<&str> {
    page.summaries.iter().map(|s| s.id.as_str()).collect()
}

#[path = "tests/binding_files.rs"]
mod binding_files;
#[path = "tests/deletion.rs"]
mod deletion;
#[path = "tests/reads_listing.rs"]
mod reads_listing;
#[path = "tests/writes.rs"]
mod writes;
