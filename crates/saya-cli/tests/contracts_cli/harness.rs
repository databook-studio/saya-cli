/// A process-unique temp root so parallel tests never share a store or scope.
fn temp_root(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "saya-contracts-cli-{label}-{}-{stamp}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

/// A single-profile runtime over a sqlite database file. The one-profile
/// connections file auto-selects `local` as the active profile, so `--profile`
/// can be omitted and still resolve. The connections path *is* the cache scope,
/// so the derived profile identity is deterministic for the leak test.
fn runtime_at(root: &Path) -> (RuntimeConfig, PathBuf, String) {
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
        connections: Some(connections.clone()),
        ..Default::default()
    };
    let runtime = load_with_sources(&options, root, root, BTreeMap::new()).unwrap();
    (runtime, connections, "local".to_string())
}

/// A store whose migrations have run, at `root/state.sqlite3`.
async fn store_at(root: &Path) -> SqliteStateStore {
    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    // Touch the pool so migrations run.
    store
        .upsert_schema(
            &identity_for(&runtime_for_scope(root), "local"),
            &SchemaTree::default(),
        )
        .await
        .unwrap();
    store
}

/// Re-derive the profile identity the adapter will compute for `name` under the
/// scope implied by `root`'s connections file. The leak test needs the *real*
/// identity, not a placeholder, because that is the string that could reach an
/// error or diagnostic if the mapping is wrong.
fn identity_for(runtime: &RuntimeConfig, name: &str) -> String {
    let profile = runtime.named_profile(name).unwrap();
    profile_identity(name, profile, &runtime.cache_scope)
        .as_str()
        .to_owned()
}

/// A throwaway runtime used only to derive a scope-matched identity for store
/// setup; the real per-test runtime is built fresh by `runtime_at`.
fn runtime_for_scope(root: &Path) -> RuntimeConfig {
    let connections = root.join("connections.toml");
    let options = saya_cli::GlobalOptions {
        connections: Some(connections),
        ..Default::default()
    };
    load_with_sources(&options, root, root, BTreeMap::new()).unwrap()
}

/// Runs `command` against `runtime`/`store` in `format`, returning the exit code
/// plus the captured (stdout, stderr).
async fn run(
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

/// Parse every non-empty captured stdout line as a JSON `TerminalEvent`.
fn json_events(stdout: &str) -> Vec<serde_json::Value> {
    stdout
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_str(line).expect("line is JSON"))
        .collect()
}

fn qualified() -> &'static str {
    "analytics.public.orders"
}

/// A table-level payload whose value, kind, origin and status the round-trip
/// test asserts appear in rendered output.
fn alias_payload() -> ClaimPayload {
    ClaimPayload::table_alias("customers").unwrap()
}

/// Propose a candidate directly through the store, so `review --confirm` has
/// something that is *not* already confirmed to act on. `remember` only stores
/// confirmed claims, so candidates must be seeded out-of-band.
///
/// Seeds a `Pending` knowledge item — the row the migrated `queue`/`review`/
/// `show`/`list` all read — and returns its `ki-…` id, the id those commands
/// render and `review`/`decide` resolve. The write path is `knowledge_items`
/// only now, so there is no legacy `contract_claims` row to seed alongside it.
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
    let payload = alias_payload();
    seed_pending_item(store, &object, &payload, ClaimOrigin::AssistantInferred).await
}

/// The "no schema observed" fingerprint the headless adapter stores: current
/// format, all-zero digest, guaranteed never to equal a real schema's digest so
/// a later live schema never reads the claim as `current`.
fn unobserved_fingerprint() -> SchemaFingerprint {
    SchemaFingerprint::from_parts(saya_types::FINGERPRINT_VERSION, &"0".repeat(64)).unwrap()
}

/// The slot a table-alias payload files under — the only kind the candidate
/// seeds below produce. Mirrors the `slot_for` pairing in the in-crate
/// `contracts/tests.rs`, trimmed to what these seeds use.
fn slot_for(payload: &ClaimPayload) -> KnowledgeSlot {
    match payload {
        ClaimPayload::TableAlias { .. } => KnowledgeSlot::TableAlias,
        _ => panic!("candidate seed only handles TableAlias, got {payload:?}"),
    }
}

/// Seeds a `Pending` knowledge item for `object` under the slot `payload` files
/// into, with the unobserved fingerprint (current version) and a `Table`
/// binding derived from the payload, and returns its `ki-…` id.
///
/// The migrated `queue`/`review`/`show` commands read `knowledge_items`, not the
/// legacy `contract_claims` the seeds used to write alone — so a seed that only
/// wrote the legacy row was invisible to them (the split brain this file's
/// `queue_*` tests hit). This mirrors the `put_item` helper in the in-crate
/// `contracts/tests.rs` and the knowledge half of `seed_current_claim` below.
async fn seed_pending_item(
    store: &SqliteStateStore,
    object: &DatabaseObjectRef,
    payload: &ClaimPayload,
    source: ClaimOrigin,
) -> ClaimId {
    let slot = slot_for(payload);
    let binding = SchemaBinding::derive(&slot, payload).expect("slot/payload agree");
    store
        .put_knowledge_item(KnowledgeItemRequest {
            object: object.clone(),
            slot: slot.clone(),
            value: payload.clone(),
            source,
            state: KnowledgeState::Pending,
            schema_binding_json: serde_json::to_string(&binding).unwrap(),
            fingerprint: unobserved_fingerprint(),
        })
        .await
        .unwrap();
    let id = store
        .knowledge_for_object(object)
        .await
        .expect("knowledge items listed")
        .into_iter()
        .find(|i| i.slot == slot)
        .expect("seeded item present")
        .id;
    ClaimId::parse(&id).expect("ki id parses")
}

// ---------------------------------------------------------------------------
// 1. remember then show round-trips value, kind, origin, status
// ---------------------------------------------------------------------------
