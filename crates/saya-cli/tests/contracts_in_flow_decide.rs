//! Spec D — decide about a claim where it appears: act on a claim from the turn
//! that just showed it by a short on-screen reference (the stored claim-id
//! prefix `contracts list` already abbreviates), not a 64-character id.
//!
//! These tests drive the shared headless dispatcher (`run_contracts`) with the
//! new `ContractsCommand::Decide` the slash adapter translates `/confirm` and
//! `/reject` into, and assert the binding-intent behaviours in spec §5. The harness mirrors `contracts_slash_parity.rs` so the derived
//! profile identity and store shape match the rest of the suite.
//!
//! The short reference is the stored `ClaimId` prefix, NOT a per-turn index. An
//! index recycles every turn, so a user who reads a receipt, thinks, runs another
//! query and then types `/confirm 2` would silently write the wrong claim. An id
//! is immutable: a stale prefix resolves to the same claim or refuses.
//!
//! `/use` is deliberately absent from the slash surface. `use_candidate_once`
//! exists and is unit-tested, but the interactive session does not yet thread the
//! admission into the next recall, so the command could not do what its name
//! promises.

use saya_cli::{
    ContractsCommand, ForgetReasonArg, RenderFormat, RuntimeConfig, SlashCommand,
    capture_output_start, capture_output_take, load_with_sources, parse_slash_command,
    profile_identity, run_contracts,
};
use saya_store::{KnowledgeItemRequest, KnowledgeItemStore, SchemaStore, SqliteStateStore};
use saya_types::{
    ClaimId, ClaimOrigin, ClaimPayload, DatabaseObjectKind, DatabaseObjectRef, KnowledgeSlot,
    KnowledgeState, ProfileIdentity, SchemaBinding, SchemaFingerprint, SchemaTree,
};
use std::{
    collections::BTreeMap, fs, path::Path, path::PathBuf, time::SystemTime, time::UNIX_EPOCH,
};

// ---------------------------------------------------------------------------
// harness — mirrors tests/contracts_slash_parity.rs so the derived profile
// identity is identical across suites and the stored records match.
// ---------------------------------------------------------------------------

fn temp_root(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "saya-in-flow-{label}-{}-{stamp}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

fn runtime_at(root: &Path) -> (RuntimeConfig, String) {
    let database = root.join("data.sqlite3");
    fs::write(&database, b"").unwrap();
    let connections = root.join("connections.toml");
    fs::write(
        &connections,
        format!(
            "[profiles.local]\ntype = 'sqlite'\npath = '{}'\n",
            database.display()
        ),
    )
    .unwrap();
    let options = saya_cli::GlobalOptions {
        connections: Some(connections),
        ..Default::default()
    };
    let runtime = load_with_sources(&options, root, root, BTreeMap::new()).unwrap();
    (runtime, "local".to_string())
}

async fn store_at(root: &Path) -> SqliteStateStore {
    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    store
        .upsert_schema(
            &identity_for(&runtime_for_scope(root), "local"),
            &SchemaTree::default(),
        )
        .await
        .unwrap();
    store
}

fn runtime_for_scope(root: &Path) -> RuntimeConfig {
    let connections = root.join("connections.toml");
    let options = saya_cli::GlobalOptions {
        connections: Some(connections),
        ..Default::default()
    };
    load_with_sources(&options, root, root, BTreeMap::new()).unwrap()
}

fn identity_for(runtime: &RuntimeConfig, name: &str) -> String {
    let profile = runtime.named_profile(name).unwrap();
    profile_identity(name, profile, &runtime.cache_scope)
        .as_str()
        .to_owned()
}

async fn run_headless(
    command: ContractsCommand,
    runtime: &RuntimeConfig,
    store: &SqliteStateStore,
    format: RenderFormat,
) -> (i32, String, String) {
    capture_output_start();
    let code = run_contracts(command, runtime, format, store)
        .await
        .unwrap();
    let (out, err) = capture_output_take();
    (code, out, err)
}

/// Parses a slash line to its `ContractsCommand` (the slash adapter's only job),
/// then runs it through the same dispatcher.
async fn run_slash(
    line: &str,
    runtime: &RuntimeConfig,
    store: &SqliteStateStore,
    format: RenderFormat,
) -> (ContractsCommand, i32, String, String) {
    let command = match parse_slash_command(line) {
        Ok(Some(SlashCommand::Contracts(cmd))) => cmd,
        other => panic!("expected SlashCommand::Contracts for {line:?}, got {other:?}"),
    };
    let (code, out, err) = run_headless(command.clone(), runtime, store, format).await;
    (command, code, out, err)
}

fn qualified() -> &'static str {
    "analytics.public.orders"
}

/// The "no schema observed" fingerprint the headless adapter stores.
fn unobserved_fingerprint() -> SchemaFingerprint {
    SchemaFingerprint::from_parts(saya_types::FINGERPRINT_VERSION, &"0".repeat(64)).unwrap()
}

/// Propose a candidate claim directly through the store, so `Decide` has a
/// not-yet-confirmed claim to act on. `remember` only stores confirmed claims.
///
/// Seeds a `Pending` knowledge item — the row the `Decide` resolve step, the
/// queue, and `show` all read — and returns its `ki-…` id, the id those
/// commands render and `Decide` resolves by prefix. The write path is
/// `knowledge_items` only now, so there is no legacy `c-` id to return.
async fn seed_candidate(store: &SqliteStateStore, runtime: &RuntimeConfig, table: &str) -> ClaimId {
    let identity = identity_for(runtime, "local");
    let profile = ProfileIdentity::parse(&identity).unwrap();
    let object = DatabaseObjectRef::new(
        profile,
        "analytics",
        "public",
        table,
        DatabaseObjectKind::Table,
    )
    .unwrap();
    let payload = ClaimPayload::table_alias(table).unwrap();
    let slot = KnowledgeSlot::TableAlias;
    let binding = SchemaBinding::derive(&slot, &payload).expect("slot/payload agree");
    store
        .put_knowledge_item(KnowledgeItemRequest {
            object: object.clone(),
            slot,
            value: payload,
            source: ClaimOrigin::AssistantInferred,
            state: KnowledgeState::Pending,
            schema_binding_json: serde_json::to_string(&binding).unwrap(),
            fingerprint: unobserved_fingerprint(),
        })
        .await
        .unwrap();
    let id = store
        .knowledge_for_object(&object)
        .await
        .expect("knowledge items listed")
        .into_iter()
        .find(|i| i.slot == KnowledgeSlot::TableAlias)
        .expect("alias item stored")
        .id;
    ClaimId::parse(&id).expect("ki id parses")
}

/// The short reference the user types: the stored claim-id prefix
/// (`ki-` + first 3 hex chars), matching `render_contract::abbreviate_id`'s
/// 6-char display prefix (which keeps `ki-` + 3 hex of a `ki-` id).
fn short_prefix(id: &ClaimId) -> String {
    // `abbreviate_id` keeps the first CLAIM_ID_PREFIX (6) chars when the id is
    // longer than 7. A knowledge-item id is `ki-` + 64 hex (67 chars), so the
    // on-screen reference is the first 6 chars. The user may type more to
    // disambiguate; the resolution matches by prefix, so the displayed 6 chars
    // always work.
    id.as_str().chars().take(6).collect()
}

// ---------------------------------------------------------------------------
// 1. Confirming by the short reference confirms exactly the intended claim.
// ---------------------------------------------------------------------------
include!("contracts_in_flow_decide/confirm_reject.rs");

// ---------------------------------------------------------------------------
// 4. An ambiguous or unresolvable reference refuses, and changes nothing.
// ---------------------------------------------------------------------------
include!("contracts_in_flow_decide/prefix_safety.rs");

// ---------------------------------------------------------------------------
// 6. Usage errors never contain the user's argument text.
// ---------------------------------------------------------------------------
include!("contracts_in_flow_decide/usage_compatibility.rs");
