//! Render-owned view DTOs for contract import and export (slice 6b).
//!
//! Presentation types only, like the other contract view DTOs. `profile` is the
//! profile *name*, never the opaque identity — the export bytes carry the
//! qualified object name only, and these DTOs reflect what the adapter rendered,
//! not the file contents. There is no field for the opaque identity on any DTO
//! here, by the same structural guarantee as `ContractView`.

use serde::{Deserialize, Serialize};

/// One claim's import verdict, in renderable form. `status` carries the
/// existing claim's real status for a duplicate (so a duplicate of a forgotten
/// claim reads forgotten), and the existing claim id for a conflict; both are
/// absent for `added`/`stale`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContractImportClaimView {
    pub verdict: String,
    pub object: String,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub existing_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub existing_id: Option<String>,
}

/// The import report: the four per-claim verdicts plus discovery's rejected
/// files and truncation flag. `dry_run` is true when nothing was written.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContractImportView {
    pub profile: String,
    pub dry_run: bool,
    pub claims: Vec<ContractImportClaimView>,
    pub rejected: Vec<(String, String)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub truncated_by: Option<String>,
}

/// The export report: the files written and a count of `Relationship` claims
/// skipped because the v1 discovered shape cannot represent them (reported, not
/// silently dropped).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContractExportView {
    pub profile: String,
    pub written: Vec<String>,
    pub skipped_relationship: usize,
}
