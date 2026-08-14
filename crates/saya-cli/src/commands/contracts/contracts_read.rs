//! Read-only `contracts` commands: `list` and `show`. Both resolve a profile,
//! call a `crate::contracts` read operation, and map the result to a render DTO.
//! An unopenable store is a diagnostic on this path, not a crash (plan §12).

use super::contracts_map::contract_view;
use super::contracts_profile::resolve_profile;
use super::{ArgMessage, EXIT_CONTRACT_ERROR, arg_failure, op_failure, unobserved_fingerprint};
use crate::commands::output::{emit, failure_message, result};
use crate::config::runtime::RuntimeConfig;
use crate::contracts::args::parse_qualified;
use crate::contracts::{
    RecallBounds, RecallMode, RecallRequest, recall, review_queue, show as show_contract,
};
use crate::render::{RenderFormat, TerminalEvent};
use saya_store::{ContractStore, SqliteStateStore};
use saya_types::{DatabaseObjectKind, DatabaseObjectRef};

// An unreadable store is not an empty store. `list` exits non-zero so "you have
// no contracts" and "I could not read your contracts" stay distinguishable —
// the same reason `show` propagates its error. Plan section 12's rule that
// memory failure must not degrade the ordinary path governs the agent query
// path, not a command whose only job is reading contracts.
const STORE_UNAVAILABLE_MSG: &str = "Local state store unavailable; contracts could not be read.";

pub(super) async fn list(
    store: &SqliteStateStore,
    runtime: &RuntimeConfig,
    format: RenderFormat,
    profile: Option<&str>,
) -> Result<i32, Box<dyn std::error::Error>> {
    let (name, identity) = match resolve_profile(runtime, profile) {
        Ok(value) => value,
        Err((code, message)) => return failure_message(code, message, format),
    };
    // `list` shows every contract for the profile. Recall selects by query, so
    // seed the query with the profile's own objects as explicit refs — the
    // selection, bounds, privacy and validity rules still all run inside recall.
    let objects = match store.list_objects(&identity).await {
        Ok(objects) => objects,
        Err(_) => {
            return failure_message(EXIT_CONTRACT_ERROR, STORE_UNAVAILABLE_MSG.into(), format);
        }
    };
    let explicit_refs: Vec<DatabaseObjectRef> = objects.iter().map(|o| o.object.clone()).collect();
    let request = RecallRequest {
        profiles: std::slice::from_ref(&identity),
        explicit_refs: &explicit_refs,
        terms: &[],
        // The user is reading their own local store; nothing is sent to a
        // provider, so the database-context privacy gate does not apply here.
        allow_database_context: true,
        schemas: &[],
        bounds: RecallBounds::defaults(),
        // `contracts list` shows confirmed contracts — the review queue is the
        // view for candidates, so the list command does not widen to them.
        recall_mode: RecallMode::Confirmed,
    };
    let outcome = recall(store, request).await;
    if outcome.diagnostics.store_unavailable {
        return failure_message(EXIT_CONTRACT_ERROR, STORE_UNAVAILABLE_MSG.into(), format);
    }
    let contracts: Vec<_> = outcome
        .contracts
        .iter()
        .map(|c| contract_view(c, &name))
        .collect();
    emit(TerminalEvent::ContractList { contracts }, format);
    Ok(0)
}

pub(super) async fn show(
    store: &SqliteStateStore,
    runtime: &RuntimeConfig,
    format: RenderFormat,
    table: &str,
    profile: Option<&str>,
) -> Result<i32, Box<dyn std::error::Error>> {
    let (name, identity) = match resolve_profile(runtime, profile) {
        Ok(value) => value,
        Err((code, message)) => return failure_message(code, message, format),
    };
    let qualified = match parse_qualified(table) {
        Ok(q) => q,
        Err(_) => return arg_failure(ArgMessage::MalformedTable, format),
    };
    let object = match DatabaseObjectRef::new(
        identity,
        &qualified.catalog,
        &qualified.schema,
        &qualified.object,
        DatabaseObjectKind::Table,
    ) {
        Ok(object) => object,
        Err(_) => return arg_failure(ArgMessage::MalformedTable, format),
    };
    let retrieved = match show_contract(store, &object, &unobserved_fingerprint()).await {
        Ok(retrieved) => retrieved,
        Err(error) => return op_failure(error, format),
    };
    let Some(contract) = retrieved else {
        return result(
            format!("No contract for {}.", object.qualified_name()),
            format,
        );
    };
    emit(
        TerminalEvent::ContractShow {
            contract: contract_view(&contract, &name),
        },
        format,
    );
    Ok(0)
}

/// The candidate review queue. Resolves a profile, lists its candidates, and
/// renders them with the schema state and evidence count a reviewer needs. The
/// headless adapter has no live schema, so each candidate reads
/// `live_schema_unavailable` — the same posture as `show`'s
/// `LiveSchemaUnavailable`. An unreadable store exits non-zero, matching `list`
/// so "no candidates" and "could not read your candidates" stay distinguishable.
pub(super) async fn queue(
    store: &SqliteStateStore,
    runtime: &RuntimeConfig,
    format: RenderFormat,
    profile: Option<&str>,
    limit: Option<usize>,
) -> Result<i32, Box<dyn std::error::Error>> {
    let (name, identity) = match resolve_profile(runtime, profile) {
        Ok(value) => value,
        Err((code, message)) => return failure_message(code, message, format),
    };
    let limit = limit.unwrap_or(crate::contracts::QUEUE_DEFAULT_LIMIT);
    let queued = match review_queue(store, std::slice::from_ref(&identity), &[], limit).await {
        Ok(queued) => queued,
        Err(_) => {
            return failure_message(EXIT_CONTRACT_ERROR, STORE_UNAVAILABLE_MSG.into(), format);
        }
    };
    let items: Vec<_> = queued
        .iter()
        .map(|c| super::queue_item_view(c, &name))
        .collect();
    emit(TerminalEvent::ContractQueue { items }, format);
    Ok(0)
}
