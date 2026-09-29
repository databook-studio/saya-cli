//! The headless `saya contracts export|import|import-dbt` adapter (B2c): the
//! file/path layer around the typed `crate::contracts::portable` operations.
//!
//! This module resolves the profile, validates the document or manifest
//! (before any store access, so an invalid file writes nothing), chooses the
//! schema the mapping runs against — the cache when one holds a real tree, a
//! live fetch when the cache is missing or empty, named in the report — calls
//! the operations, and shapes the report. Every verdict (inserted, skipped,
//! conflict, unavailable) is listed per item; the queue pointer, the preview
//! posture, and the memory-off note end it. No policy lives here:
//! pending-not-active, conflict-not-overwrite, and every bound are the
//! operations' and the store's decisions.

use std::path::Path;

use super::contracts_profile::resolve_profile;
use super::{EXIT_CONTRACT_ERROR, cached_schema_availability};
use crate::commands::connection;
use crate::commands::output::{emit, failure_message};
use crate::config::runtime::RuntimeConfig;
use crate::contracts::SchemaAvailability;
use crate::contracts::dbt::parse_dbt_manifest;
use crate::contracts::portable;
use crate::render::{RenderFormat, TerminalEvent};
use saya_config::MemoryMode;
use saya_store::SqliteStateStore;
use saya_types::{ContextItem, ProfileIdentity, SchemaTree};

use super::contracts_portable_report::{DbtProvenance, export_report, import_report};

pub(super) async fn export(
    store: &SqliteStateStore,
    runtime: &RuntimeConfig,
    format: RenderFormat,
    path: &Path,
    profile: Option<&str>,
    overwrite: bool,
) -> Result<i32, Box<dyn std::error::Error>> {
    let (_name, identity) = match resolve_profile(runtime, profile) {
        Ok(value) => value,
        Err((code, message)) => return failure_message(code, message, format),
    };
    match portable::export(store, &identity, path, overwrite).await {
        Ok(outcome) => {
            emit(
                TerminalEvent::Result {
                    message: export_report(&outcome),
                },
                format,
            );
            Ok(0)
        }
        Err(error) => fail(error.to_string(), format),
    }
}

pub(super) async fn import(
    store: &SqliteStateStore,
    runtime: &RuntimeConfig,
    format: RenderFormat,
    path: &Path,
    profile: Option<&str>,
    preview: bool,
) -> Result<i32, Box<dyn std::error::Error>> {
    // The whole document validates before any store access.
    let items = match portable::read_context(path) {
        Ok(document) => document.items,
        Err(error) => return fail(error.to_string(), format),
    };
    run_import(store, runtime, format, &items, profile, preview, None).await
}

pub(super) async fn import_dbt(
    store: &SqliteStateStore,
    runtime: &RuntimeConfig,
    format: RenderFormat,
    manifest: &Path,
    profile: Option<&str>,
    select: &[String],
    preview: bool,
) -> Result<i32, Box<dyn std::error::Error>> {
    // The manifest parses and its items validate before any store access.
    let parsed = match parse_dbt_manifest(manifest, select) {
        Ok(parsed) => parsed,
        Err(error) => return fail(error.to_string(), format),
    };
    let skipped: Vec<(String, String)> = parsed
        .skipped
        .iter()
        .map(|(unique_id, reason)| (unique_id.clone(), (*reason).to_string()))
        .collect();
    let provenance = DbtProvenance {
        version: parsed.version.as_str(),
        skipped: &skipped,
    };
    run_import(
        store,
        runtime,
        format,
        &parsed.items,
        profile,
        preview,
        Some(&provenance),
    )
    .await
}

/// The shared import flow: map the validated items against the chosen schema,
/// apply the batch, and report. Exits 2 when items were offered and none
/// landed; a preview always exits 0.
async fn run_import(
    store: &SqliteStateStore,
    runtime: &RuntimeConfig,
    format: RenderFormat,
    items: &[ContextItem],
    profile: Option<&str>,
    preview: bool,
    provenance: Option<&DbtProvenance<'_>>,
) -> Result<i32, Box<dyn std::error::Error>> {
    let (name, identity) = match resolve_profile(runtime, profile) {
        Ok(value) => value,
        Err((code, message)) => return failure_message(code, message, format),
    };
    let (tree, source) = match schema_for(store, runtime, &name, &identity).await {
        Ok(value) => value,
        Err(message) => return fail(message, format),
    };
    let outcome = match portable::import_document(store, &identity, items, &tree, preview).await {
        Ok(outcome) => outcome,
        Err(error) => return fail(error.to_string(), format),
    };
    let memory_off = runtime.resolved.memory.mode == MemoryMode::Off;
    let message = import_report(&name, source, &outcome, preview, provenance, memory_off);
    emit(TerminalEvent::Result { message }, format);
    // "Offered and none landed" counts the items the parser skipped too: a dbt
    // manifest whose every item failed mapping landed nothing.
    let offered = outcome.total() + provenance.map(|p| p.skipped.len()).unwrap_or(0);
    let nothing_landed = !preview && offered > 0 && outcome.landed() == 0;
    Ok(if nothing_landed {
        EXIT_CONTRACT_ERROR
    } else {
        0
    })
}

/// The schema the mapping resolves against: a real cached tree, or a live
/// fetch when the cache is missing or empty. The report names which — the two
/// must stay distinguishable, the way [`SchemaAvailability`] insists.
async fn schema_for(
    store: &SqliteStateStore,
    runtime: &RuntimeConfig,
    name: &str,
    identity: &ProfileIdentity,
) -> Result<(SchemaTree, &'static str), String> {
    match cached_schema_availability(store, identity).await {
        SchemaAvailability::Available { schema, .. } if !schema.databases.is_empty() => {
            Ok((schema, "cached"))
        }
        SchemaAvailability::Unavailable => {
            Err("Local state store unavailable; the schema cache could not be read.".to_string())
        }
        _ => {
            let profile = runtime
                .named_profile(name)
                .map_err(|error| format!("could not load the profile: {error}"))?;
            let connector = connection::build(profile, runtime, false)
                .await
                .map_err(|error| format!("could not connect to fetch the schema: {error}"))?;
            connector
                .connect()
                .await
                .map_err(|error| format!("could not connect to fetch the schema: {error}"))?;
            let schema = connector
                .schema()
                .await
                .map_err(|error| format!("could not fetch the schema: {error}"))?;
            Ok((schema, "live"))
        }
    }
}

fn fail(message: String, format: RenderFormat) -> Result<i32, Box<dyn std::error::Error>> {
    failure_message(EXIT_CONTRACT_ERROR, message, format)
}
