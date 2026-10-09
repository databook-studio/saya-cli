//! Cross-adapter parity for the Phase 2b-4 slash adapters: the `/contracts`,
//! `/contract`, `/remember` and `/forget` slash commands must call the *same*
//! operations as the headless `saya contracts` commands and add nothing — no
//! second parsing, no second privacy decision, no second DTO mapping.
//!
//! Detection, not demonstration: each test runs the slash path and the headless
//! path against the same store + profile and asserts they agree on the thing
//! that would diverge if a second code path had snuck in (claim ids and order,
//! the retrieved contract, the stored claim, the tombstone, the rejection
//! class). If a helper were copied and drifted, one of these fails.
//!
//! Both paths converge on `run_contracts(ContractsCommand, …)`: the slash path
//! only translates slash text into a `ContractsCommand` and hands it to the same
//! dispatcher. The parity test proves the translated value is equivalent to the
//! headless one and that both produce identical results.

use saya_cli::{
    ClaimKindArg, ContractsCommand, ForgetReasonArg, RenderFormat, RuntimeConfig, SlashCommand,
    capture_output_start, capture_output_take, load_with_sources, parse_slash_command,
    profile_identity, run_contracts,
};
use saya_store::{KnowledgeItemStore, SchemaStore, SqliteStateStore};
use saya_types::{
    ClaimOrigin, ClaimPayload, DatabaseObjectKind, DatabaseObjectRef, KnowledgeSlot,
    KnowledgeState, ProfileIdentity, SchemaBinding, SchemaFingerprint, SchemaTree,
};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

// ---------------------------------------------------------------------------
// harness — mirrors tests/contracts_cli.rs so the two suites share one store
// shape and the derived profile identity is identical across them.
// ---------------------------------------------------------------------------

fn temp_root(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "saya-slash-parity-{label}-{}-{stamp}",
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
    store_at_name(root, "state.sqlite3").await
}

/// A migrated store at `root/<name>`, sharing `root`'s connections file (so the
/// derived profile identity matches any other store built from the same root).
async fn store_at_name(root: &Path, name: &str) -> SqliteStateStore {
    let store = SqliteStateStore::new(root.join(name));
    let runtime = runtime_for_scope(root);
    let identity = identity_for(&runtime, "local");
    store
        .upsert_schema(&identity, &SchemaTree::default())
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

/// Runs a `ContractsCommand` through the shared headless dispatcher and returns
/// the exit code plus captured (stdout, stderr).
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
/// then runs it through the same dispatcher. Returns the parsed command (for
/// parity assertions on the translated value) and the captured output.
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

/// The "no schema observed" fingerprint the headless adapter stores: current
/// format, all-zero digest. The parity test seeds candidates with it so their
/// stored fingerprint matches what the headless `unobserved_fingerprint` would
/// produce — keeping the two paths' records identical.
fn unobserved_fingerprint() -> SchemaFingerprint {
    SchemaFingerprint::from_parts(saya_types::FINGERPRINT_VERSION, &"0".repeat(64)).unwrap()
}

// ---------------------------------------------------------------------------
// 1. /contracts and `saya contracts list` produce the same claim ids, ordered.
// ---------------------------------------------------------------------------

include!("contracts_slash_parity/list_show.rs");
include!("contracts_slash_parity/remember_forget.rs");
include!("contracts_slash_parity/invalid_and_roundtrip.rs");
include!("contracts_slash_parity/usage_store_errors.rs");
include!("contracts_slash_parity/remember_translation.rs");
include!("contracts_slash_parity/queue_parity.rs");
