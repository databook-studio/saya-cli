//! Import and export `contracts` commands (slice 6b). Each resolves a profile,
//! calls the typed `crate::contracts::io` operation, maps the result to a
//! render DTO, and emits it. An unreadable store is a genuine failure and exits
//! non-zero — the user asked for something that did not happen — matching the
//! read/write commands' posture.

use super::contracts_profile::resolve_profile;
use super::{EXIT_CONTRACT_ERROR, op_failure};
use crate::commands::output::{emit, failure_message};
use crate::contracts::io::{ImportClaimResult, ImportVerdict};
use crate::contracts::{export_contracts, import_contracts};
use crate::render::{
    ContractExportView, ContractImportClaimView, ContractImportView, RenderFormat, TerminalEvent,
};
use saya_store::SqliteStateStore;
use std::path::Path;

pub(super) async fn import(
    store: &SqliteStateStore,
    runtime: &crate::config::runtime::RuntimeConfig,
    format: RenderFormat,
    project_root: &Path,
    dry_run: bool,
    profile: Option<&str>,
) -> Result<i32, Box<dyn std::error::Error>> {
    let (name, identity) = match resolve_profile(runtime, profile) {
        Ok(value) => value,
        Err((code, message)) => return failure_message(code, message, format),
    };
    let report = match import_contracts(store, &identity, project_root, dry_run).await {
        Ok(report) => report,
        Err(error) => return op_failure(error, format),
    };
    let claims = report
        .added
        .iter()
        .chain(&report.duplicates)
        .chain(&report.conflicts)
        .chain(&report.stale)
        .map(claim_view)
        .collect();
    let view = ContractImportView {
        profile: name,
        dry_run,
        claims,
        rejected: report
            .rejected
            .iter()
            .map(|(p, r)| (p.display().to_string(), r.clone()))
            .collect(),
        truncated_by: report.truncated_by.map(Into::into),
    };
    emit(TerminalEvent::ContractImport { report: view }, format);
    Ok(0)
}

pub(super) async fn export(
    store: &SqliteStateStore,
    runtime: &crate::config::runtime::RuntimeConfig,
    format: RenderFormat,
    destination: &Path,
    overwrite: bool,
    profile: Option<&str>,
) -> Result<i32, Box<dyn std::error::Error>> {
    let (name, identity) = match resolve_profile(runtime, profile) {
        Ok(value) => value,
        Err((code, message)) => return failure_message(code, message, format),
    };
    let outcome = match export_contracts(store, &identity, destination, overwrite).await {
        Ok(outcome) => outcome,
        Err(error) => return op_failure(error, format),
    };
    let view = ContractExportView {
        profile: name,
        written: outcome
            .written
            .iter()
            .map(|p| p.display().to_string())
            .collect(),
        skipped_relationship: outcome.skipped_relationship,
    };
    emit(TerminalEvent::ContractExport { report: view }, format);
    Ok(0)
}

/// Maps one import claim result to its render DTO, preserving the existing
/// claim's status (duplicate) or id (conflict) so the report names what the
/// file collided with.
fn claim_view(result: &ImportClaimResult) -> ContractImportClaimView {
    let (existing_status, existing_id) = match &result.verdict {
        ImportVerdict::Duplicate { existing_status } => (Some(existing_status.clone()), None),
        ImportVerdict::Conflicting { existing_id } => (None, Some(existing_id.clone())),
        _ => (None, None),
    };
    ContractImportClaimView {
        verdict: result.verdict.as_str().into(),
        object: result.object.clone(),
        source: result.source.display().to_string(),
        existing_status,
        existing_id,
    }
}

// Silence the unused-import lint for `EXIT_CONTRACT_ERROR`, which the import/
// export paths do not reach (they route through `op_failure` for store errors
// and `failure_message` for profile resolution) but which the sibling command
// modules share this file's error-code convention with.
#[allow(dead_code)]
const _EXIT_CONTRACT_ERROR: i32 = EXIT_CONTRACT_ERROR;
