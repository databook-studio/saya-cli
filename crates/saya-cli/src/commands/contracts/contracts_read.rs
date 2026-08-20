//! Read-only `contracts` commands: `list` and `show`. Both resolve a profile,
//! call a `crate::contracts` read operation, and map the result to a render DTO.
//! An unopenable store is a diagnostic on this path, not a crash (plan §12).

use super::contracts_map::contract_view;
use super::contracts_profile::resolve_profile;
use super::{ArgMessage, EXIT_CONTRACT_ERROR, arg_failure, cached_schema_availability, op_failure};
use crate::commands::output::{emit, failure_message, result};
use crate::config::runtime::RuntimeConfig;
use crate::contracts::args::parse_qualified;
use crate::contracts::{
    RecallBounds, RecallMode, RecallRequest, RetrievalPolicy, now_unix_ms, recall, review_queue,
    show as show_contract,
};
use crate::render::{RenderFormat, TerminalEvent};
use saya_store::{KnowledgeItemStore, SqliteStateStore};
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
    // The object list comes from `knowledge_items` (the same table `show`/`queue`
    // read), so a `remember`-written fact is listable the same turn — no split
    // brain with the legacy `contract_objects` table the old `list_objects` read.
    let explicit_refs: Vec<DatabaseObjectRef> = match store.objects_for_profile(&identity).await {
        Ok(objects) => objects,
        Err(_) => {
            return failure_message(EXIT_CONTRACT_ERROR, STORE_UNAVAILABLE_MSG.into(), format);
        }
    };
    // The cached schema classifies each claim — what `connection schema
    // --refresh` wrote. `Missing` and `Unavailable` stay distinct (not collapsed
    // to an empty tree) so the honest `live_schema_unavailable` is preserved.
    let cached = cached_schema_availability(store, &identity).await;
    let schema_pair = (identity.clone(), cached);
    let schemas = std::slice::from_ref(&schema_pair);
    let request = RecallRequest {
        profiles: std::slice::from_ref(&identity),
        explicit_refs: &explicit_refs,
        terms: &[],
        // The user is reading their own local store; nothing is sent to a
        // provider, so the database-context privacy gate does not apply here.
        allow_database_context: true,
        schemas,
        // `now_unix_ms` is unused on the human path (`ForHumanReview` is
        // unbounded), but the field is required; pass the real clock.
        now_unix_ms: now_unix_ms(),
        bounds: RecallBounds::defaults(),
        // `contracts list` shows confirmed contracts — the review queue is the
        // view for candidates, so the list command does not widen to them.
        recall_mode: RecallMode::Confirmed,
        // No per-claim admission on the human list path; `None` keeps the mode.
        admit_candidate: None,
        // A human is reviewing; keep stale contracts so the list stays a true
        // picture of what is stored. The model-facing recall path is the one
        // that drops. Unbounded freshness: a stale-by-age cache still shows
        // what it knows — a reviewer is not asked to trust a query built on it.
        policy: RetrievalPolicy::ForHumanReview,
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
        identity.clone(),
        &qualified.catalog,
        &qualified.schema,
        &qualified.object,
        DatabaseObjectKind::Table,
    ) {
        Ok(object) => object,
        Err(_) => return arg_failure(ArgMessage::MalformedTable, format),
    };
    // `show` classifies against the cached schema, the same source `list` uses;
    // `Missing`/`Unavailable` stay distinct so the honest `live_schema_unavailable`
    // is preserved, not fabricated into `current` or `stale`.
    let cached = cached_schema_availability(store, &identity).await;
    // `contracts show` is a human-review path: keep a stale contract, its
    // fingerprints and the reason it is stale — that is what the reviewer is
    // here to act on. The model-facing `contract_read` is the path that drops.
    // `now_unix_ms` is unused here (`ForHumanReview` is unbounded), but the
    // field is required; pass the real clock.
    let retrieved = match show_contract(
        store,
        &object,
        &cached,
        RetrievalPolicy::ForHumanReview,
        now_unix_ms(),
    )
    .await
    {
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

/// The candidate review queue. Resolves a profile, lists its candidates and
/// stale claims, and renders them with the schema state and evidence count a
/// reviewer needs. The queue classifies against the cached schema — the same
/// source `list`/`show` use, via the shared `cached_schema` helper — so a
/// candidate made right after a `connection schema --refresh` reads a real
/// state, not a constant `live_schema_unavailable`. A missing cache stays
/// `None` so the honest `live_schema_unavailable` is preserved. An unreadable
/// store exits non-zero, matching `list` so "no candidates" and "could not read
/// your candidates" stay distinguishable.
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
    // The cached schema classifies each queued claim — the same source `list`
    // and `show` read. `Missing`/`Unavailable` stay distinct (not an empty
    // tree) so a candidate made right after a refresh reads a real state, and a
    // store error reads the honest `live_schema_unavailable`.
    let cached = cached_schema_availability(store, &identity).await;
    let schema_pair = (identity.clone(), cached);
    let schemas = std::slice::from_ref(&schema_pair);
    let queued = match review_queue(store, std::slice::from_ref(&identity), schemas, limit).await {
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
