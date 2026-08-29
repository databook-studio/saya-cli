//! Headless `saya contracts` commands: drives the command adapter directly
//! (constructs `ContractsCommand` values and calls `run_contracts`), never
//! shelling out to a built binary. Covers the spec at
//! `.claude/specs/spec-2b2c-contract-commands.md`.
//!
//! Output is captured through the thread-local seam in `output::emit` so the
//! tests can assert on rendered text/JSON/NDJSON without touching the process
//! stdout (which would race under parallel test runs).

use saya_cli::{
    ClaimKindArg, Cli, Command, ContractsCommand, ForgetReasonArg, RenderFormat, ReviewDecisionArg,
    RuntimeConfig, capture_output_start, capture_output_take, load_with_sources, profile_identity,
    run_contracts,
};
use saya_store::{KnowledgeItemRequest, KnowledgeItemStore, SchemaStore, SqliteStateStore};
use saya_types::{
    ClaimId, ClaimOrigin, ClaimPayload, Column, ColumnRequirement, Database, DatabaseObjectKind,
    DatabaseObjectRef, FINGERPRINT_VERSION, KnowledgeSlot, KnowledgeState, ProfileIdentity, Schema,
    SchemaBinding, SchemaFingerprint, SchemaTree, Table,
};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

// ---------------------------------------------------------------------------
// harness
// ---------------------------------------------------------------------------

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
#[tokio::test]
async fn remember_then_show_round_trips_claim_fields() {
    let root = temp_root("round_trip");
    let (runtime, _connections, _name) = runtime_at(&root);
    let store = store_at(&root).await;

    let remember = ContractsCommand::Remember {
        table: qualified().into(),
        kind: ClaimKindArg::Alias,
        value: "customers".into(),
        column: None,
        reason: None,
        profile: None,
    };
    let (code, out, err) = run(remember, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "stderr: {err}");
    assert!(out.contains("remembered"), "out: {out}");
    assert!(out.contains("confirmed"), "out: {out}");

    let show = ContractsCommand::Show {
        table: qualified().into(),
        profile: None,
    };
    let (code, out, err) = run(show, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "stderr: {err}");
    assert!(out.contains("customers"), "value missing: {out}");
    assert!(out.contains("table_alias"), "kind missing: {out}");
    assert!(out.contains("user_explicit"), "origin missing: {out}");
    assert!(out.contains("confirmed"), "status missing: {out}");

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 2. remember twice -> duplicate with the same fact, no second claim
// ---------------------------------------------------------------------------
#[tokio::test]
async fn remember_twice_reports_duplicate_with_same_id() {
    let root = temp_root("duplicate");
    let (runtime, _c, _n) = runtime_at(&root);
    let store = store_at(&root).await;

    let remember = ContractsCommand::Remember {
        table: qualified().into(),
        kind: ClaimKindArg::Alias,
        value: "customers".into(),
        column: None,
        reason: None,
        profile: None,
    };
    let (code1, out1, err1) = run(remember.clone(), &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code1, 0, "stderr: {err1}");
    assert!(
        out1.contains("remembered alias customers for analytics.public.orders (confirmed)"),
        "confirmation names fact and object in words: {out1}"
    );
    assert!(!out1.contains("ki-"), "must contain no raw id: {out1}");

    let (code2, out2, err2) = run(remember, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code2, 0, "stderr: {err2}");
    assert!(
        out2.contains(
            "duplicate of alias customers for analytics.public.orders — already exists (confirmed)"
        ),
        "second must report duplicate naming fact and object: {out2}"
    );
    assert!(
        !out2.contains("ki-"),
        "duplicate must contain no raw id: {out2}"
    );

    // Exactly one knowledge item exists for the object — the duplicate wrote
    // nothing, so the row count did not grow.
    let identity = identity_for(&runtime, "local");
    let profile = ProfileIdentity::parse(&identity).unwrap();
    let object = DatabaseObjectRef::new(
        profile,
        "analytics",
        "public",
        "orders",
        DatabaseObjectKind::Table,
    )
    .unwrap();
    let items = store.knowledge_for_object(&object).await.unwrap();
    assert_eq!(
        items.len(),
        1,
        "duplicate created a second knowledge item: {items:?}"
    );

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 2b. re-remembering a directive claim with a NEW reason revises it: the
// value is unchanged (so the single-valued slot does not move), but the
// reason the user just stated is written — a user who explained themselves
// must not be ignored. A genuinely identical re-remember (same value AND
// same reason) is still a no-op duplicate. (Open Question 2.)
// ---------------------------------------------------------------------------
#[tokio::test]
async fn remember_again_with_a_new_reason_revises_the_reason() {
    let root = temp_root("reason_revision");
    let (runtime, _c, _n) = runtime_at(&root);
    let store = store_at(&root).await;

    // First remember: a time-column with a reason.
    let first = ContractsCommand::Remember {
        table: qualified().into(),
        kind: ClaimKindArg::TimeColumn,
        value: "return_date".into(),
        column: None,
        reason: Some("a rental only counts once it comes back".into()),
        profile: None,
    };
    let (code1, out1, err1) = run(first, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code1, 0, "stderr: {err1}");
    assert!(out1.contains("remembered"), "first remember: {out1}");

    // Second remember: same value, a NEW reason. This is a revision, not a
    // duplicate — the reason is written.
    let second = ContractsCommand::Remember {
        table: qualified().into(),
        kind: ClaimKindArg::TimeColumn,
        value: "return_date".into(),
        column: None,
        reason: Some("rentals are counted on return for billing".into()),
        profile: None,
    };
    let (code2, out2, err2) = run(second, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code2, 0, "stderr: {err2}");
    assert!(
        out2.contains("remembered"),
        "a new reason revises the claim, not a duplicate: {out2}"
    );
    assert!(
        !out2.contains("duplicate"),
        "a new reason must not read as a duplicate: {out2}"
    );

    // Exactly one row, and it carries the NEW reason.
    let identity = identity_for(&runtime, "local");
    let profile = ProfileIdentity::parse(&identity).unwrap();
    let object = DatabaseObjectRef::new(
        profile,
        "analytics",
        "public",
        "orders",
        DatabaseObjectKind::Table,
    )
    .unwrap();
    let items = store.knowledge_for_object(&object).await.unwrap();
    assert_eq!(items.len(), 1, "revision wrote no new row: {items:?}");
    assert!(
        matches!(
            &items[0].value,
            saya_types::ClaimPayload::DefaultTimeColumn { column, reason, .. }
            if column == "return_date"
            && reason.as_deref() == Some("rentals are counted on return for billing")
        ),
        "the stored reason is the new one: {:?}",
        items[0].value
    );

    // A third remember with the SAME value and SAME reason is a no-op duplicate.
    let third = ContractsCommand::Remember {
        table: qualified().into(),
        kind: ClaimKindArg::TimeColumn,
        value: "return_date".into(),
        column: None,
        reason: Some("rentals are counted on return for billing".into()),
        profile: None,
    };
    let (code3, out3, err3) = run(third, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code3, 0, "stderr: {err3}");
    assert!(
        out3.contains("duplicate"),
        "identical re-remember is a duplicate: {out3}"
    );

    // A fourth remember with the SAME value and NO reason does NOT erase the
    // stored reason — silence is not "drop the reason," only a *new* reason
    // revises. It reports a duplicate, and the row keeps its reason.
    let fourth = ContractsCommand::Remember {
        table: qualified().into(),
        kind: ClaimKindArg::TimeColumn,
        value: "return_date".into(),
        column: None,
        reason: None,
        profile: None,
    };
    let (code4, out4, err4) = run(fourth, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code4, 0, "stderr: {err4}");
    assert!(
        out4.contains("duplicate"),
        "re-stating without a reason is a duplicate, not an erase: {out4}"
    );
    let items_again = store.knowledge_for_object(&object).await.unwrap();
    assert!(matches!(
        &items_again[0].value,
        saya_types::ClaimPayload::DefaultTimeColumn { reason, .. }
        if reason.as_deref() == Some("rentals are counted on return for billing")
    ));

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 3. forget then show no longer lists the claim
// ---------------------------------------------------------------------------
#[tokio::test]
async fn forget_then_show_no_longer_lists_the_claim() {
    let root = temp_root("forget");
    let (runtime, _c, _n) = runtime_at(&root);
    let store = store_at(&root).await;

    let remember = ContractsCommand::Remember {
        table: qualified().into(),
        kind: ClaimKindArg::Alias,
        value: "customers".into(),
        column: None,
        reason: None,
        profile: None,
    };
    let (code, out, err) = run(remember, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "stderr: {err}");
    assert!(
        out.contains("remembered alias customers for analytics.public.orders (confirmed)"),
        "names fact and object: {out}"
    );
    assert!(!out.contains("ki-"), "no raw id in remember output: {out}");

    let identity = identity_for(&runtime, "local");
    let profile = ProfileIdentity::parse(&identity).unwrap();
    let object = DatabaseObjectRef::new(
        profile,
        "analytics",
        "public",
        "orders",
        DatabaseObjectKind::Table,
    )
    .unwrap();
    let items = store.knowledge_for_object(&object).await.unwrap();
    let id = items[0].id.as_str();

    let forget = ContractsCommand::Forget {
        claim_id: id.into(),
        reason: ForgetReasonArg::Incorrect,
    };
    let (code, out, err) = run(forget, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "stderr: {err}");
    assert!(out.contains("forgotten"), "out: {out}");

    let show = ContractsCommand::Show {
        table: qualified().into(),
        profile: None,
    };
    let (code, out, err) = run(show, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "stderr: {err}");
    assert!(
        !out.contains("customers"),
        "forgotten claim still listed: {out}"
    );

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 4. remember, forget, then remember again -> duplicate whose status is forgotten
// ---------------------------------------------------------------------------
#[tokio::test]
async fn remember_after_forget_reports_duplicate_forgotten_not_success() {
    let root = temp_root("reforget");
    let (runtime, _c, _n) = runtime_at(&root);
    let store = store_at(&root).await;

    let remember = ContractsCommand::Remember {
        table: qualified().into(),
        kind: ClaimKindArg::Alias,
        value: "customers".into(),
        column: None,
        reason: None,
        profile: None,
    };
    let (code, out, err) = run(remember.clone(), &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "stderr: {err}");
    assert!(out.contains("remembered"), "out: {out}");
    assert!(!out.contains("ki-"), "no raw id: {out}");

    let identity = identity_for(&runtime, "local");
    let profile = ProfileIdentity::parse(&identity).unwrap();
    let object = DatabaseObjectRef::new(
        profile,
        "analytics",
        "public",
        "orders",
        DatabaseObjectKind::Table,
    )
    .unwrap();
    let items = store.knowledge_for_object(&object).await.unwrap();
    let id = items[0].id.as_str();

    let forget = ContractsCommand::Forget {
        claim_id: id.into(),
        reason: ForgetReasonArg::Incorrect,
    };
    let (code, _out, err) = run(forget, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "stderr: {err}");

    // Re-remembering the same value hits the tombstone's dedup key and must read
    // as a duplicate of a *forgotten* claim, not as a fresh success or a new id.
    let (code, out, err) = run(remember, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "stderr: {err}");
    assert!(out.contains("duplicate"), "out: {out}");
    assert!(
        out.contains("previously forgotten"),
        "duplicate of a forgotten claim must say previously forgotten: {out}"
    );
    assert!(
        out.contains("alias customers for analytics.public.orders"),
        "names fact and object: {out}"
    );
    assert!(!out.contains("ki-"), "no raw id: {out}");
    assert!(
        !out.starts_with("remembered"),
        "must not read as success: {out}"
    );

    let identity = identity_for(&runtime, "local");
    let profile = ProfileIdentity::parse(&identity).unwrap();
    let object = DatabaseObjectRef::new(
        profile,
        "analytics",
        "public",
        "orders",
        DatabaseObjectKind::Table,
    )
    .unwrap();
    let items = store.knowledge_for_object(&object).await.unwrap();
    assert_eq!(
        items.len(),
        1,
        "re-remember created a new knowledge item: {items:?}"
    );
    // The duplicate wrote nothing, so the tombstone stayed dismissed — a
    // re-remember of a forgotten fact does not silently revive it.
    assert_eq!(
        items[0].state,
        KnowledgeState::Dismissed,
        "the forgotten tombstone must stay dismissed: {items:?}"
    );

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 5. decide --decision confirm: a `Pending` candidate becomes `Active`
//    (confirmed); re-confirming the now-`Active` item against a valid cached
//    schema *revalidates* and stays `Active` (confirm is idempotent —
//    re-confirming is how a user asks "is this still true?"); confirming a
//    `Dismissed` item is a typed conflict (a withdrawn fact is not revivable
//    by revalidation). This is the same confirm op `review --confirm` reached;
//    `decide` resolves the prefix to the id and forwards to it.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn decide_confirm_on_candidate_confirms_and_on_confirmed_conflicts() {
    let root = temp_root("decide_confirm");
    let (runtime, _c, _n) = runtime_at(&root);
    let store = store_at(&root).await;
    // Cache a schema that names `orders` so a re-confirm of the now-`Active`
    // alias has a live table to revalidate against (the alias's `Table`
    // binding is valid whenever the table exists). `store_at` cached an empty
    // default; overwrite it for this profile.
    let identity = identity_for(&runtime, "local");
    store
        .upsert_schema(&identity, &orders_schema())
        .await
        .unwrap();

    let candidate_id = seed_candidate(&store, &runtime, "orders").await;
    // The full stored id is a prefix of itself, so `decide` resolves it to
    // exactly one claim (spec invariant 1a).
    let prefix = candidate_id.as_str().to_string();

    // 1. Pending → Active: the candidate is confirmed.
    let confirm = ContractsCommand::Decide {
        prefix: prefix.clone(),
        decision: ReviewDecisionArg::Confirm,
        profile: None,
    };
    let (code, out, err) = run(confirm, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "stderr: {err}");
    assert!(out.contains("confirmed"), "candidate -> confirmed: {out}");

    // 2. Re-confirming the now-Active item against the valid cached schema
    //    revalidates and stays Active — confirm is idempotent, not a conflict.
    //    A re-confirm is how a user asks "is this still true?", and refusing
    //    would leave them no way to re-check a fact they suspect has drifted.
    let (code, out, err) = run(
        ContractsCommand::Decide {
            prefix: prefix.clone(),
            decision: ReviewDecisionArg::Confirm,
            profile: None,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(code, 0, "re-confirm must succeed, not conflict: {out}{err}");
    assert!(
        out.contains("confirmed"),
        "re-confirmed item stays Active/confirmed: {out}"
    );

    // 3. Confirming a `Dismissed` item is a typed conflict — a withdrawn fact
    //    is not revivable by revalidation, so confirm refuses rather than
    //    silently reviving it.
    let dismissed_id = seed_candidate(&store, &runtime, "shipments").await;
    let _ = run(
        ContractsCommand::Forget {
            claim_id: dismissed_id.as_str().into(),
            reason: ForgetReasonArg::Incorrect,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    let (code, out, err) = run(
        ContractsCommand::Decide {
            prefix: dismissed_id.as_str().into(),
            decision: ReviewDecisionArg::Confirm,
            profile: None,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_ne!(
        code, 0,
        "confirming a dismissed item must not succeed: {out}{err}"
    );
    let combined = format!("{out}{err}");
    assert!(
        combined.contains("conflict"),
        "expected a typed conflict for a dismissed item, got: {combined}"
    );

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 6. unknown --profile -> typed error listing available names, non-zero
// ---------------------------------------------------------------------------
#[tokio::test]
async fn unknown_profile_is_typed_error_listing_available_names() {
    let root = temp_root("unknown_profile");
    let (runtime, _c, _n) = runtime_at(&root);
    let store = store_at(&root).await;

    let list = ContractsCommand::List {
        profile: Some("nope".into()),
    };
    let (code, out, err) = run(list, &runtime, &store, RenderFormat::Text).await;
    assert_ne!(code, 0, "unknown profile must not succeed: {out}{err}");
    let combined = format!("{out}{err}");
    assert!(
        combined.contains("nope"),
        "error should name the unknown profile: {combined}"
    );
    assert!(
        combined.contains("local"),
        "error should list available profile names: {combined}"
    );
    // The identity must never reach an error string either.
    let identity = identity_for(&runtime, "local");
    assert!(
        !combined.contains(&identity),
        "opaque identity leaked into error: {combined}"
    );

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 7. malformed qualified table -> typed error naming the expected form
// ---------------------------------------------------------------------------
#[tokio::test]
async fn malformed_qualified_table_is_typed_error() {
    let root = temp_root("malformed_table");
    let (runtime, _c, _n) = runtime_at(&root);
    let store = store_at(&root).await;

    for bad in ["orders", "public.orders", "a.b.c.d"] {
        let show = ContractsCommand::Show {
            table: bad.into(),
            profile: None,
        };
        let (code, out, err) = run(show, &runtime, &store, RenderFormat::Text).await;
        assert_ne!(code, 0, "{bad:?} must not succeed: {out}{err}");
        let combined = format!("{out}{err}");
        assert!(
            combined.contains("catalog.schema.object"),
            "error should name the expected form for {bad:?}: {combined}"
        );
        assert!(
            !combined.contains(bad),
            "error must not echo untrusted input {bad:?}: {combined}"
        );
    }

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 8. the opaque identity appears in no output (text, JSON, NDJSON)
// ---------------------------------------------------------------------------
#[tokio::test]
async fn opaque_profile_identity_never_reaches_rendered_output() {
    let root = temp_root("identity_leak");
    let (runtime, _c, _n) = runtime_at(&root);
    let store = store_at(&root).await;

    let remember = ContractsCommand::Remember {
        table: qualified().into(),
        kind: ClaimKindArg::Alias,
        value: "customers".into(),
        column: None,
        reason: None,
        profile: None,
    };
    let (code, _out, err) = run(remember, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "stderr: {err}");

    let identity = identity_for(&runtime, "local");
    assert_eq!(identity.len(), 66);
    assert!(identity.starts_with("p-"));

    let list = ContractsCommand::List { profile: None };
    let show = ContractsCommand::Show {
        table: qualified().into(),
        profile: None,
    };
    for format in [RenderFormat::Text, RenderFormat::Json, RenderFormat::Ndjson] {
        let (code, list_out, list_err) = run(list.clone(), &runtime, &store, format).await;
        assert_eq!(code, 0, "list {format:?} stderr: {list_err}");
        let (code, show_out, show_err) = run(show.clone(), &runtime, &store, format).await;
        assert_eq!(code, 0, "show {format:?} stderr: {show_err}");
        for captured in [list_out, list_err, show_out, show_err] {
            assert!(
                !captured.contains(&identity),
                "opaque identity leaked into {format:?} output: {captured}"
            );
        }
    }

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 9. An unreadable store is not an empty store. `list` exits non-zero so that
//    "you have no contracts" and "I could not read your contracts" stay
//    distinguishable, matching `show`. Writes also fail.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn unopenable_store_reads_and_writes_both_exit_nonzero() {
    let root = temp_root("unopenable");
    // A path whose parent is a regular file cannot be created as a directory,
    // so the store pool cannot open.
    fs::write(root.join("blocker"), b"x").unwrap();
    let bad = root.join("blocker/state.sqlite3");
    let store = SqliteStateStore::new(&bad);

    let (runtime, _c, _n) = runtime_at(&root);

    let list = ContractsCommand::List { profile: None };
    let (code, out, err) = run(list, &runtime, &store, RenderFormat::Text).await;
    assert_ne!(
        code, 0,
        "list must exit non-zero on an unopenable store: {out}{err}"
    );
    let combined = format!("{out}{err}");
    assert!(
        !combined.is_empty(),
        "list must emit a diagnostic on an unopenable store"
    );

    let remember = ContractsCommand::Remember {
        table: qualified().into(),
        kind: ClaimKindArg::Alias,
        value: "customers".into(),
        column: None,
        reason: None,
        profile: None,
    };
    let (code, _out, _err) = run(remember, &runtime, &store, RenderFormat::Text).await;
    assert_ne!(
        code, 0,
        "a write against an unopenable store must exit non-zero"
    );

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 10. JSON and NDJSON carry the same claim ids as the text form
// ---------------------------------------------------------------------------
#[tokio::test]
async fn json_and_ndjson_carry_same_claim_ids_as_text() {
    let root = temp_root("formats");
    let (runtime, _c, _n) = runtime_at(&root);
    let store = store_at(&root).await;

    let remember = ContractsCommand::Remember {
        table: qualified().into(),
        kind: ClaimKindArg::Alias,
        value: "customers".into(),
        column: None,
        reason: None,
        profile: None,
    };
    // The text `remember` line confirms the fact in words without raw id.
    let (code, text_out, err) = run(remember.clone(), &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "stderr: {err}");
    assert!(
        text_out.contains("remembered alias customers for analytics.public.orders"),
        "text form names the fact and object: {text_out}"
    );
    assert!(
        !text_out.contains("ki-"),
        "text form drops raw id: {text_out}"
    );

    let identity = identity_for(&runtime, "local");
    let profile = ProfileIdentity::parse(&identity).unwrap();
    let object = DatabaseObjectRef::new(
        profile,
        "analytics",
        "public",
        "orders",
        DatabaseObjectKind::Table,
    )
    .unwrap();
    let items = store.knowledge_for_object(&object).await.unwrap();
    let stored_id = items[0].id.as_str();

    // The duplicate event (same value remembered again) in JSON and NDJSON
    // carries `contract_remembered`.
    for format in [RenderFormat::Json, RenderFormat::Ndjson] {
        let (code, out, err) = run(remember.clone(), &runtime, &store, format).await;
        assert_eq!(code, 0, "{format:?} stderr: {err}");
        let events = json_events(&out);
        let remembered_evt = events
            .iter()
            .find(|v| v["event"] == "contract_remembered")
            .expect("a contract_remembered event was emitted");
        assert_eq!(remembered_evt["action"], "duplicate");
        assert_eq!(
            remembered_evt["status"], "confirmed",
            "duplicate of a confirmed claim carries its status"
        );
        assert_eq!(remembered_evt["object"], "analytics.public.orders");
        assert_eq!(remembered_evt["kind"], "alias");
        assert_eq!(remembered_evt["value"], "customers");
        // The id left the text confirmation but not the machine surface: a
        // script that remembers then forgets still has a handle.
        assert_eq!(
            remembered_evt["claim_id"], stored_id,
            "{format:?} remember id disagrees with stored id"
        );
    }

    // …and the same command rendered as text names the fact instead of the id.
    let (code, remember_text, err) =
        run(remember.clone(), &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "stderr: {err}");
    assert!(
        !remember_text.contains(stored_id),
        "text confirmation must not print the raw id: {remember_text}"
    );

    // `show` carries the claim id in every structured format (JSON/NDJSON),
    // which matches the store's id.
    let show = ContractsCommand::Show {
        table: qualified().into(),
        profile: None,
    };
    for format in [RenderFormat::Json, RenderFormat::Ndjson] {
        let (code, out, err) = run(show.clone(), &runtime, &store, format).await;
        assert_eq!(code, 0, "{format:?} stderr: {err}");
        let events = json_events(&out);
        let shown = events
            .iter()
            .find(|v| v["event"] == "contract_show")
            .expect("a contract_show event was emitted");
        let claim_id = shown["contract"]["claims"][0]["claim_id"]
            .as_str()
            .expect("claim id present");
        assert_eq!(
            claim_id, stored_id,
            "{format:?} show id disagrees with stored id"
        );
    }

    // The text `show` line contains the value, so the round-trip is real.
    let (code, show_text, err) = run(show, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "stderr: {err}");
    assert!(show_text.contains("customers"), "text show: {show_text}");

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 11. Phase 3d: `saya contracts queue` lists candidates, and confirming one
//     from the queue makes it recallable while rejecting it never does. The
//     confirm reaches the existing `confirm` op through `decide` (the survivor
//     of the `review` retirement) — no new write tool.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn queue_lists_candidates_and_decide_transitions_them() {
    let root = temp_root("queue_decide");
    let (runtime, _c, _n) = runtime_at(&root);
    let store = store_at(&root).await;
    // `store_at` cached an empty default schema for this profile; drop it so the
    // candidate genuinely has no cached schema and the queue reads the honest
    // `live_schema_unavailable` — matching how `list`/`show`'s no-cache test sets
    // up. An empty cached tree would otherwise read `stale` (the object is
    // absent from it), which is correct but not what this test exercises.
    let identity = identity_for(&runtime, "local");
    store.invalidate_schema(&identity).await.unwrap();

    let cand_id = seed_candidate(&store, &runtime, "orders").await;
    let queue = ContractsCommand::Queue {
        profile: None,
        limit: None,
    };
    let (code, out, err) = run(queue, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "queue stderr: {err}");
    // The queue is a worklist: each entry carries the decision a reviewer is
    // being asked to make as a status word. A fresh candidate reads
    // `candidate`; a persisted stale claim reads `stale`. The fields a
    // reviewer needs are the id, the status, the kind/value, the object, the
    // schema state, and the evidence count.
    assert!(
        out.contains(cand_id.as_str()),
        "queue must name the candidate's full id: {out}"
    );
    assert!(out.contains("candidate"), "queue must show status: {out}");
    assert!(out.contains("table_alias"), "queue must show kind: {out}");
    assert!(out.contains("customers"), "queue must show value: {out}");
    assert!(
        out.contains("analytics.public.orders"),
        "queue must show object: {out}"
    );
    assert!(
        out.contains("live_schema_unavailable"),
        "queue must show schema state: {out}"
    );
    assert!(
        out.contains("evidence 0"),
        "queue must show evidence count: {out}"
    );

    // Confirm the candidate: it becomes recallable (show lists it) and leaves
    // the queue on the next read. `decide` resolves the full id (a prefix of
    // itself) to the candidate and forwards to the `confirm` op.
    let confirm = ContractsCommand::Decide {
        prefix: cand_id.as_str().into(),
        decision: ReviewDecisionArg::Confirm,
        profile: None,
    };
    let (code, out, err) = run(confirm, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "confirm stderr: {err}");
    assert!(out.contains("confirmed"), "confirm out: {out}");

    let show = ContractsCommand::Show {
        table: qualified().into(),
        profile: None,
    };
    let (code, out, err) = run(show, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "show stderr: {err}");
    assert!(
        out.contains("customers"),
        "confirmed candidate is now recallable: {out}"
    );

    let queue = ContractsCommand::Queue {
        profile: None,
        limit: None,
    };
    let (code, out, err) = run(queue, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "queue stderr: {err}");
    assert!(
        !out.contains(cand_id.as_str()),
        "confirmed candidate must leave the queue: {out}"
    );

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn queue_rejected_candidate_never_becomes_recallable() {
    let root = temp_root("queue_reject");
    let (runtime, _c, _n) = runtime_at(&root);
    let store = store_at(&root).await;

    let cand_id = seed_candidate(&store, &runtime, "orders").await;
    let reject = ContractsCommand::Decide {
        prefix: cand_id.as_str().into(),
        decision: ReviewDecisionArg::Reject,
        profile: None,
    };
    let (code, out, err) = run(reject, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "reject stderr: {err}");
    assert!(out.contains("rejected"), "reject out: {out}");

    // The rejected candidate left the queue…
    let queue = ContractsCommand::Queue {
        profile: None,
        limit: None,
    };
    let (code, out, err) = run(queue, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "queue stderr: {err}");
    assert!(
        !out.contains(cand_id.as_str()),
        "rejected candidate must leave the queue: {out}"
    );

    // …and never becomes recallable: show finds no contract for the object.
    let show = ContractsCommand::Show {
        table: qualified().into(),
        profile: None,
    };
    let (code, out, err) = run(show, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "show stderr: {err}");
    assert!(
        out.contains("No contract"),
        "rejected candidate must never be recallable: {out}"
    );

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn queue_never_leaks_the_opaque_profile_identity() {
    let root = temp_root("queue_identity");
    let (runtime, _c, _n) = runtime_at(&root);
    let store = store_at(&root).await;

    let _id = seed_candidate(&store, &runtime, "orders").await;
    let identity = identity_for(&runtime, "local");
    let queue = ContractsCommand::Queue {
        profile: None,
        limit: None,
    };
    for format in [RenderFormat::Text, RenderFormat::Json, RenderFormat::Ndjson] {
        let (code, out, err) = run(queue.clone(), &runtime, &store, format).await;
        assert_eq!(code, 0, "queue {format:?} stderr: {err}");
        for captured in [out.as_str(), err.as_str()] {
            assert!(
                !captured.contains(&identity),
                "opaque identity leaked into queue {format:?}: {captured}"
            );
        }
    }

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn queue_unopenable_store_exits_nonzero_like_list() {
    let root = temp_root("queue_unopenable");
    fs::write(root.join("blocker"), b"x").unwrap();
    let bad = root.join("blocker/state.sqlite3");
    let store = SqliteStateStore::new(&bad);
    let (runtime, _c, _n) = runtime_at(&root);

    let queue = ContractsCommand::Queue {
        profile: None,
        limit: None,
    };
    let (qcode, qout, qerr) = run(queue, &runtime, &store, RenderFormat::Text).await;
    assert_ne!(
        qcode, 0,
        "queue must exit non-zero on an unopenable store: {qout}{qerr}"
    );
    assert!(
        !format!("{qout}{qerr}").is_empty(),
        "queue must emit a diagnostic on an unopenable store"
    );

    // The failure matches `list` — "no candidates" and "could not read" stay
    // distinguishable the same way "no contracts" and "unreadable" do.
    let list = ContractsCommand::List { profile: None };
    let (lcode, lout, lerr) = run(list, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(qcode, lcode, "queue exit diverged from list");
    assert_eq!(qout, lout, "queue stdout diverged from list");
    assert_eq!(qerr, lerr, "queue stderr diverged from list");

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 12. `contracts list` / `show` report schema state from the cached schema,
//     not a constant `live_schema_unavailable`. Regression for a bug found by
//     exercising the built binary: both commands passed `schemas: &[]` (list)
//     and `unobserved_fingerprint()` (show) and never loaded the cached schema,
//     so validity could only ever compute LiveSchemaUnavailable — even right
//     after a `connection schema --refresh` that populated the cache. Every
//     unit test built the schemas explicitly, so no test caught it.
//
//     The claim is seeded with the *real* fingerprint of the cached table (not
//     the unobserved sentinel `remember` stores), so a matching cache reads
//     `current`. When no schema is cached, `live_schema_unavailable` stays the
//     honest answer — that path is asserted too, so the fix cannot regress to
//     fabricating `current` from an empty cache.
// ---------------------------------------------------------------------------
fn orders_table() -> Table {
    Table {
        name: "orders".into(),
        columns: vec![Column {
            name: "id".into(),
            data_type: "bigint".into(),
            nullable: false,
        }],
    }
}

fn orders_schema() -> SchemaTree {
    SchemaTree {
        databases: vec![Database {
            name: "analytics".into(),
            schemas: vec![Schema {
                name: "public".into(),
                tables: vec![orders_table()],
            }],
        }],
    }
}

/// Seeds a confirmed (Active) knowledge item for `orders` whose
/// `SchemaBinding` the cached `orders` table satisfies, so a matching cache
/// classifies it `current`. The item is written under the current fingerprint
/// version with a `Table` binding (a table-alias depends only on the table
/// existing), which is the shape a `remember`-written alias takes.
async fn seed_current_claim(store: &SqliteStateStore, runtime: &RuntimeConfig) {
    let identity = identity_for(runtime, "local");
    let profile = ProfileIdentity::parse(&identity).unwrap();
    let object = DatabaseObjectRef::new(
        profile,
        "analytics",
        "public",
        "orders",
        DatabaseObjectKind::Table,
    )
    .unwrap();
    let payload = ClaimPayload::table_alias("orders").unwrap();
    let slot = KnowledgeSlot::TableAlias;
    let binding = SchemaBinding::derive(&slot, &payload).expect("slot/payload agree");
    store
        .put_knowledge_item(KnowledgeItemRequest {
            object: object.clone(),
            slot,
            value: payload,
            source: ClaimOrigin::UserExplicit,
            state: KnowledgeState::Active,
            schema_binding_json: serde_json::to_string(&binding).unwrap(),
            // Current version; the digest is not persisted on the row (only the
            // version is), so the all-zero sentinel stands in — `item_validity_for`
            // classifies via the binding + version, not a stored digest.
            fingerprint: SchemaFingerprint::from_parts(FINGERPRINT_VERSION, &"0".repeat(64))
                .unwrap(),
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn list_and_show_report_current_against_a_cached_schema() {
    let root = temp_root("list_show_cached_schema_state");
    let (runtime, _c, _n) = runtime_at(&root);
    let store = store_at(&root).await;
    seed_current_claim(&store, &runtime).await;
    // Cache the schema that matches the seeded claim — what
    // `connection schema <profile> --refresh` would have written.
    let identity = identity_for(&runtime, "local");
    store
        .upsert_schema(&identity, &orders_schema())
        .await
        .unwrap();

    // `list` must report `current`, not `live_schema_unavailable`.
    let list = ContractsCommand::List { profile: None };
    let (code, out, err) = run(list, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "list stderr: {err}");
    assert!(
        out.contains("[current]"),
        "list must report current against the cached schema: {out}"
    );
    assert!(
        !out.contains("live_schema_unavailable"),
        "list must not report live_schema_unavailable when a schema is cached: {out}"
    );

    // `show` must report `current` for the same reason.
    let show = ContractsCommand::Show {
        table: qualified().into(),
        profile: None,
    };
    let (code, out, err) = run(show, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "show stderr: {err}");
    assert!(
        out.contains("[current]"),
        "show must report current against the cached schema: {out}"
    );
    assert!(
        !out.contains("live_schema_unavailable"),
        "show must not report live_schema_unavailable when a schema is cached: {out}"
    );

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn list_and_show_report_live_schema_unavailable_when_nothing_is_cached() {
    let root = temp_root("list_show_no_cache");
    let (runtime, _c, _n) = runtime_at(&root);
    // A store whose only cached schema is the empty default `store_at` writes
    // for *its* profile — invalidate it so there is genuinely no cache.
    let store = store_at(&root).await;
    let identity = identity_for(&runtime, "local");
    store.invalidate_schema(&identity).await.unwrap();
    seed_current_claim(&store, &runtime).await;

    // With no cached schema, `live_schema_unavailable` is the honest answer —
    // not a fallback the fix fabricates a `current` from.
    let list = ContractsCommand::List { profile: None };
    let (code, out, err) = run(list, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "list stderr: {err}");
    assert!(
        out.contains("live_schema_unavailable"),
        "list must report live_schema_unavailable when no schema is cached: {out}"
    );

    let show = ContractsCommand::Show {
        table: qualified().into(),
        profile: None,
    };
    let (code, out, err) = run(show, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "show stderr: {err}");
    assert!(
        out.contains("live_schema_unavailable"),
        "show must report live_schema_unavailable when no schema is cached: {out}"
    );

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 13. Symptom B regression: a claim remembered right AFTER a schema refresh
//     reads `current`, not `needs_review`. Before the fix `remember` stored the
//     all-zero unobserved fingerprint, which can never equal a real digest, so
//     every claim read `needs_review — the schema changed since the claim was
//     made` even though nothing changed. Now `remember` computes the real
//     `of_table` digest against the cached table and stores typed column
//     snapshots, so the claim matches the cache.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn remember_against_cached_schema_reads_current_not_needs_review() {
    let root = temp_root("remember_current_not_needs_review");
    let (runtime, _c, _n) = runtime_at(&root);
    let store = store_at(&root).await;
    // Cache the schema — what `connection schema local --refresh` would write.
    // `store_at` cached an empty default for its own scope; overwrite it for
    // this profile's identity with the real orders schema.
    let identity = identity_for(&runtime, "local");
    store
        .upsert_schema(&identity, &orders_schema())
        .await
        .unwrap();

    // Remember AFTER the refresh. Before the fix this stored the all-zeros
    // sentinel; now it must store the real digest of the cached orders table.
    let remember = ContractsCommand::Remember {
        table: qualified().into(),
        kind: ClaimKindArg::Alias,
        value: "customers".into(),
        column: None,
        reason: None,
        profile: None,
    };
    let (code, out, err) = run(remember, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "remember stderr: {err}");
    assert!(out.contains("remembered"), "out: {out}");

    // The stored item carries the current fingerprint version and a `Table`
    // binding derived from the alias slot. This is the load-bearing assertion
    // under the binding model: `item_validity_for` classifies via the binding +
    // version (not a stored digest — `knowledge_items` persists only the version),
    // so a current version + a `Table` binding the cached `orders` table satisfies
    // is what makes the claim read `current` instead of `needs_review`.
    let profile = ProfileIdentity::parse(&identity).unwrap();
    let object = DatabaseObjectRef::new(
        profile,
        "analytics",
        "public",
        "orders",
        DatabaseObjectKind::Table,
    )
    .unwrap();
    let items = store.knowledge_for_object(&object).await.unwrap();
    assert_eq!(items.len(), 1, "expected one item: {items:?}");
    assert_eq!(
        items[0].fingerprint_version, FINGERPRINT_VERSION,
        "remember must store the item under the current fingerprint version"
    );
    let binding: SchemaBinding =
        serde_json::from_str(&items[0].schema_binding_json).expect("binding deserializes");
    assert!(
        matches!(binding, SchemaBinding::Table),
        "a table-alias stores a Table binding, got {binding:?}"
    );
    assert_eq!(items[0].state, KnowledgeState::Active);

    // `show` classifies the claim against the same cached schema and must read
    // `current` — the user-facing symptom.
    let show = ContractsCommand::Show {
        table: qualified().into(),
        profile: None,
    };
    let (code, out, err) = run(show, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "show stderr: {err}");
    assert!(
        out.contains("[current]"),
        "remember-then-show must read current against the cached schema: {out}"
    );
    assert!(
        !out.contains("needs_review"),
        "a claim remembered right after a refresh must not read needs_review: {out}"
    );

    let _ = fs::remove_dir_all(root);
}

// A column-level claim remembered against a cached schema stores a
// `SchemaBinding::Column` naming the column and its semantic requirement, so
// the claim reads `current` against a cache that has the column. The binding
// model replaces the legacy typed column snapshot: a fact depends on a
// column *existing* (or being temporal/numeric for the role-bearing kinds), not
// on the connector's exact type string — so a harmless widening no longer
// reads `needs_review`. The guarantee kept: a column claim right after a
// refresh reads `current`, not `needs_review`.
#[tokio::test]
async fn remember_column_claim_against_cached_schema_snapshots_real_type() {
    let root = temp_root("remember_column_snapshot_real_type");
    let (runtime, _c, _n) = runtime_at(&root);
    let store = store_at(&root).await;
    let identity = identity_for(&runtime, "local");
    // orders table with the `id` column the claim references.
    let schema = SchemaTree {
        databases: vec![Database {
            name: "analytics".into(),
            schemas: vec![Schema {
                name: "public".into(),
                tables: vec![Table {
                    name: "orders".into(),
                    columns: vec![Column {
                        name: "id".into(),
                        data_type: "bigint".into(),
                        nullable: false,
                    }],
                }],
            }],
        }],
    };
    store.upsert_schema(&identity, &schema).await.unwrap();

    let remember = ContractsCommand::Remember {
        table: qualified().into(),
        kind: ClaimKindArg::ColumnRole,
        value: "identifier".into(),
        column: Some("id".into()),
        reason: None,
        profile: None,
    };
    let (code, out, err) = run(remember, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "remember stderr: {err}");
    assert!(out.contains("remembered"), "out: {out}");

    let profile = ProfileIdentity::parse(&identity).unwrap();
    let object = DatabaseObjectRef::new(
        profile,
        "analytics",
        "public",
        "orders",
        DatabaseObjectKind::Table,
    )
    .unwrap();
    let items = store.knowledge_for_object(&object).await.unwrap();
    assert_eq!(items.len(), 1, "{items:?}");
    // The binding names the column and its semantic requirement. An
    // `identifier` role requires the column to exist (`Exists`), so a cache
    // that has `id` reads `current`.
    let binding: SchemaBinding =
        serde_json::from_str(&items[0].schema_binding_json).expect("binding deserializes");
    assert_eq!(
        binding,
        SchemaBinding::Column {
            column: "id".into(),
            requirement: ColumnRequirement::Exists,
        },
        "a column-role claim stores a Column binding naming the column: {binding:?}"
    );
    assert_eq!(items[0].state, KnowledgeState::Active);

    let show = ContractsCommand::Show {
        table: qualified().into(),
        profile: None,
    };
    let (code, out, err) = run(show, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "show stderr: {err}");
    assert!(out.contains("[current]"), "must read current: {out}");

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 14. Symptom A regression: `remember` against an object the cached schema
//     does NOT contain must refuse at remember time — naming the object and
//     suggesting `connection schema --refresh` — and must store nothing.
//     Before the fix a typo'd catalog/schema/object stored happily as
//     confirmed and only a later refresh marked it stale.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn remember_unknown_object_against_cached_schema_refuses_and_stores_nothing() {
    let root = temp_root("remember_unknown_object_refuses");
    let (runtime, _c, _n) = runtime_at(&root);
    let store = store_at(&root).await;
    let identity = identity_for(&runtime, "local");
    store
        .upsert_schema(&identity, &orders_schema())
        .await
        .unwrap();

    // `analytics.public.ghost` is well-formed (so it passes parse/validate)
    // but absent from the cached schema — the typo/wrong-scope case.
    let remember = ContractsCommand::Remember {
        table: "analytics.public.ghost".into(),
        kind: ClaimKindArg::Alias,
        value: "nope".into(),
        column: None,
        reason: None,
        profile: None,
    };
    let (code, out, err) = run(remember, &runtime, &store, RenderFormat::Text).await;
    assert_ne!(code, 0, "remember of an unknown object must not succeed");
    let combined = format!("{out}{err}");
    assert!(
        combined.contains("analytics.public.ghost"),
        "error must name the unknown object so the user sees their typo: {combined}"
    );
    assert!(
        combined.contains("connection schema"),
        "error must suggest `connection schema --refresh`: {combined}"
    );
    assert!(
        combined.contains("--refresh"),
        "error must mention the --refresh flag: {combined}"
    );
    // The opaque identity must not reach the error string.
    assert!(
        !combined.contains(&identity),
        "opaque identity leaked into the unknown-object error: {combined}"
    );

    // Nothing was stored for the typo'd object.
    let profile = ProfileIdentity::parse(&identity).unwrap();
    let ghost = DatabaseObjectRef::new(
        profile,
        "analytics",
        "public",
        "ghost",
        DatabaseObjectKind::Table,
    )
    .unwrap();
    let items = store.knowledge_for_object(&ghost).await.unwrap();
    assert!(
        items.is_empty(),
        "refuse must store nothing for the unknown object: {items:?}"
    );

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 15. No-cache guard: with NO cached schema `remember` keeps today's behaviour
//     — it succeeds, and `show` reads `live_schema_unavailable`. Refusing would
//     make remember unusable before a first refresh, so the fix must not touch
//     this path. The binding model does not persist a digest on the row (only
//     the fingerprint *version*), so the "unobserved sentinel" the legacy row
//     carried is replaced by "current version + a `Table` binding + no schema
//     to classify against" — which reads `live_schema_unavailable` honestly.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn remember_with_no_cached_schema_keeps_sentinel_and_succeeds() {
    let root = temp_root("remember_no_cache_sentinel");
    let (runtime, _c, _n) = runtime_at(&root);
    let store = store_at(&root).await;
    // Genuinely no cache for this profile (store_at cached an empty default
    // for its own scope identity; drop it).
    let identity = identity_for(&runtime, "local");
    store.invalidate_schema(&identity).await.unwrap();

    let remember = ContractsCommand::Remember {
        table: qualified().into(),
        kind: ClaimKindArg::Alias,
        value: "customers".into(),
        column: None,
        reason: None,
        profile: None,
    };
    let (code, out, err) = run(remember, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "remember with no cache must still succeed: {err}");
    assert!(out.contains("remembered"), "out: {out}");

    let profile = ProfileIdentity::parse(&identity).unwrap();
    let object = DatabaseObjectRef::new(
        profile,
        "analytics",
        "public",
        "orders",
        DatabaseObjectKind::Table,
    )
    .unwrap();
    let items = store.knowledge_for_object(&object).await.unwrap();
    assert_eq!(items.len(), 1, "{items:?}");
    // The item is stored under the current fingerprint version with a `Table`
    // binding — the no-cache shape that reads `live_schema_unavailable` until a
    // schema is cached. (The digest is not persisted on the row; the version is
    // what `item_validity_for` gates on, alongside the binding.)
    assert_eq!(items[0].fingerprint_version, FINGERPRINT_VERSION);
    assert_eq!(items[0].state, KnowledgeState::Active);

    let show = ContractsCommand::Show {
        table: qualified().into(),
        profile: None,
    };
    let (code, out, err) = run(show, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "show stderr: {err}");
    assert!(
        out.contains("live_schema_unavailable"),
        "no-cache claim must read live_schema_unavailable, not be refused or fabricated current: {out}"
    );

    let _ = fs::remove_dir_all(root);
}

// An EMPTY cached schema (the no-op sentinel `store_at` writes, or a never-
// populated cache) carries no real schema information, so it must be treated
// the same as no cache: `remember` succeeds with the sentinel, it does not
// refuse every object as "unknown".
#[tokio::test]
async fn remember_against_empty_cached_schema_keeps_sentinel_and_succeeds() {
    let root = temp_root("remember_empty_cache_sentinel");
    let (runtime, _c, _n) = runtime_at(&root);
    let store = store_at(&root).await;
    // `store_at` already cached `SchemaTree::default()` (empty databases) for
    // this profile's identity — leave it. An empty tree is "no schema", not
    // "everything is unknown".
    let remember = ContractsCommand::Remember {
        table: qualified().into(),
        kind: ClaimKindArg::Alias,
        value: "customers".into(),
        column: None,
        reason: None,
        profile: None,
    };
    let (code, out, err) = run(remember, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(
        code, 0,
        "remember against an empty cached schema must succeed, not refuse: {err}"
    );
    assert!(out.contains("remembered"), "out: {out}");

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 16. ITEM 2 regression: `contracts queue` loads the cached schema (the same
//     helper `list`/`show` use) and reports a real schema state, not a constant
//     `live_schema_unavailable`. Before the fix the queue call site passed
//     `schemas: &[]`, so validity could only ever compute
//     LiveSchemaUnavailable — even right after a `connection schema --refresh`
//     that populated the cache. A candidate whose stored fingerprint equals
//     the cached table's digest must read `current` in the queue, the same as
//     `list`/`show`.
//
//     The claim is seeded as a *candidate* (the queue's subject) carrying the
//     real fingerprint of the cached table — not the unobserved sentinel
//     `remember` stores, which never matches and so could only prove the bug
//     half-fixed.
// ---------------------------------------------------------------------------
async fn seed_current_candidate(store: &SqliteStateStore, runtime: &RuntimeConfig) -> ClaimId {
    let identity = identity_for(runtime, "local");
    let profile = ProfileIdentity::parse(&identity).unwrap();
    let object = DatabaseObjectRef::new(
        profile,
        "analytics",
        "public",
        "orders",
        DatabaseObjectKind::Table,
    )
    .unwrap();
    let payload = ClaimPayload::table_alias("orders").unwrap();
    seed_pending_item(store, &object, &payload, ClaimOrigin::AssistantInferred).await
}

#[tokio::test]
async fn queue_reports_current_against_a_cached_schema() {
    let root = temp_root("queue_cached_schema_state");
    let (runtime, _c, _n) = runtime_at(&root);
    let store = store_at(&root).await;
    // Cache the schema that matches the seeded candidate — what
    // `connection schema <profile> --refresh` would have written.
    let identity = identity_for(&runtime, "local");
    store
        .upsert_schema(&identity, &orders_schema())
        .await
        .unwrap();
    seed_current_candidate(&store, &runtime).await;

    let queue = ContractsCommand::Queue {
        profile: None,
        limit: None,
    };
    let (code, out, err) = run(queue, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "queue stderr: {err}");
    assert!(
        out.contains("[current]"),
        "queue must report current against the cached schema: {out}"
    );
    assert!(
        !out.contains("live_schema_unavailable"),
        "queue must not report live_schema_unavailable when a schema is cached: {out}"
    );

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 17. The queue must not present an unreviewed proposal and a fact whose
//     dependency broke as the same thing. Under the new model staleness is
//     computed, not persisted: a "stale" fact is `Active` (state) whose
//     `SchemaBinding` reads `Invalid` against the cached schema. The queue
//     lists `Pending` candidates only, so the broken confirmed fact is not in
//     the queue — it is a different kind of thing, surfaced by `show` as
//     `stale`. The two are distinguishable: the candidate is queued (status
//     `candidate`), the broken fact is not queued and `show` reports it stale.
// ---------------------------------------------------------------------------
/// Seeds an `Active` `default_time_column` fact on `created_at` for `table`,
/// then caches a schema for the profile that has `table` but dropped
/// `created_at` — so the item's `Column { created_at, Time }` binding computes
/// `Invalid` against the cache. This is the new-model "stale" shape: a
/// confirmed fact whose dependency broke. Returns the item's `ki-…` id.
async fn seed_drifted_active_claim(
    store: &SqliteStateStore,
    runtime: &RuntimeConfig,
    table: &str,
) -> ClaimId {
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
    let payload = ClaimPayload::default_time_column("created_at", None).unwrap();
    let slot = KnowledgeSlot::TableDefaultTime;
    let binding = SchemaBinding::derive(&slot, &payload).expect("slot/payload agree");
    store
        .put_knowledge_item(KnowledgeItemRequest {
            object: object.clone(),
            slot,
            value: payload,
            source: ClaimOrigin::UserExplicit,
            state: KnowledgeState::Active,
            schema_binding_json: serde_json::to_string(&binding).unwrap(),
            fingerprint: SchemaFingerprint::from_parts(FINGERPRINT_VERSION, &"0".repeat(64))
                .unwrap(),
        })
        .await
        .unwrap();
    // Cache a schema that has `table` but dropped `created_at`, so the item's
    // `Column { created_at, Time }` binding reads `Invalid` (→ `stale`).
    let drifted = SchemaTree {
        databases: vec![Database {
            name: "analytics".into(),
            schemas: vec![Schema {
                name: "public".into(),
                tables: vec![Table {
                    name: table.into(),
                    columns: vec![Column {
                        name: "id".into(),
                        data_type: "bigint".into(),
                        nullable: false,
                    }],
                }],
            }],
        }],
    };
    store.upsert_schema(&identity, &drifted).await.unwrap();
    let id = store
        .knowledge_for_object(&object)
        .await
        .expect("knowledge items listed")
        .into_iter()
        .find(|i| i.slot == KnowledgeSlot::TableDefaultTime)
        .expect("drifted item stored")
        .id;
    ClaimId::parse(&id).expect("ki id parses")
}

#[tokio::test]
async fn queue_distinguishes_candidate_from_stale_in_output() {
    let root = temp_root("queue_candidate_vs_stale");
    let (runtime, _c, _n) = runtime_at(&root);
    let store = store_at(&root).await;
    // A `Pending` candidate (an unreviewed proposal) and an `Active` fact whose
    // binding computes `Invalid` (a confirmed fact whose dependency broke). The
    // queue is the candidate worklist: the candidate is queued; the broken
    // confirmed fact is not — it is surfaced by `show` as `stale`, not conflated
    // with a candidate.
    let cand_id = seed_candidate(&store, &runtime, "orders").await;
    let stale_id = seed_drifted_active_claim(&store, &runtime, "shipments").await;

    let queue = ContractsCommand::Queue {
        profile: None,
        limit: None,
    };
    let (code, out, err) = run(queue.clone(), &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "queue stderr: {err}");

    // The candidate is queued; the broken confirmed fact is not. The queue
    // does not present them as the same thing — one is a candidate to decide
    // on, the other is a drifted fact that is not pending review.
    assert!(
        out.contains(cand_id.as_str()),
        "candidate id missing from queue: {out}"
    );
    assert!(
        !out.contains(stale_id.as_str()),
        "a broken confirmed fact must not appear in the candidate queue: {out}"
    );
    assert!(
        out.contains("candidate"),
        "queue must show the candidate's status: {out}"
    );

    // `show` surfaces the broken fact as `stale` — the dependency-broke verdict
    // a reviewer acts on — so the two are told apart across the two views: the
    // queue has the candidate, `show` has the broken fact as stale. The show
    // stanza abbreviates the claim id (`ki-f35…`), so assert on the value the
    // broken fact carries and its stale state rather than the full id.
    let (code, out, err) = run(
        ContractsCommand::Show {
            table: "analytics.public.shipments".into(),
            profile: None,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(code, 0, "show stderr: {err}");
    assert!(
        out.contains("created_at"),
        "the broken fact is shown to a reviewer: {out}"
    );
    assert!(
        out.contains("[stale]"),
        "show reports the broken fact as stale, not current: {out}"
    );

    // The JSON queue carries the candidate's status so a machine reader sees
    // the same distinction.
    let (code, out, err) = run(queue, &runtime, &store, RenderFormat::Json).await;
    assert_eq!(code, 0, "queue json stderr: {err}");
    let events = json_events(&out);
    let queue_event = events
        .iter()
        .find(|v| v["event"] == "contract_queue")
        .expect("a contract_queue event was emitted");
    let items = queue_event["items"].as_array().expect("items is an array");
    let cand_item = items
        .iter()
        .find(|v| v["claim_id"] == cand_id.as_str())
        .expect("candidate item present in JSON");
    assert_eq!(
        cand_item["status"], "candidate",
        "JSON candidate status: {cand_item}"
    );
    assert!(
        !items.iter().any(|v| v["claim_id"] == stale_id.as_str()),
        "the broken confirmed fact must not be a queue item: {items:?}"
    );

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// S12 evidence — the `decide` command covers every capability `review` had, so
// retiring `review` loses nothing. Two differences matter (spec invariant 1):
//   (a) `review` took a full claim id; `decide` takes a prefix with a
//       `MIN_PREFIX_LEN` floor. A full-length id is a prefix of itself, so it
//       must resolve to exactly one claim.
//   (b) `review` was not profile-scoped; `decide` resolves the prefix against
//       the resolved profile's claims and has a `--profile` flag. A claim in a
//       non-active profile must stay reachable via `decide --profile <name>`.
// These two tests are the evidence; they are written against the *current*
// binary (before anything is removed) and must pass both before and after.
// ---------------------------------------------------------------------------

/// `decide` accepts a full-length claim id as a prefix of itself and resolves
/// it to exactly one claim (spec invariant 1a). `review` took the full id;
/// `decide` takes a prefix — a full id is the degenerate prefix that matches
/// only itself.
#[tokio::test]
async fn decide_accepts_a_full_length_claim_id_as_a_prefix_of_itself() {
    let root = temp_root("decide_full_id_prefix");
    let (runtime, _c, _n) = runtime_at(&root);
    let store = store_at(&root).await;
    // Cache a schema that names `orders` so the confirm revalidation (confirm
    // is idempotent on an Active alias whose table exists) succeeds.
    let identity = identity_for(&runtime, "local");
    store
        .upsert_schema(&identity, &orders_schema())
        .await
        .unwrap();

    let candidate_id = seed_candidate(&store, &runtime, "orders").await;
    // The FULL stored id (a `ki-…` of 67 chars), not an abbreviation.
    let full_id = candidate_id.as_str().to_string();

    let decide = ContractsCommand::Decide {
        prefix: full_id.clone(),
        decision: ReviewDecisionArg::Confirm,
        profile: None,
    };
    let (code, out, err) = run(decide, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "a full-length id must resolve via decide: {err}");
    assert!(out.contains("confirmed"), "candidate -> confirmed: {out}");

    // It resolved to *exactly* one claim: the candidate is now Active, and no
    // other item changed. There was only one seeded item, so this is the
    // "exactly one" guarantee the prefix floor exists to give a full id.
    let profile = ProfileIdentity::parse(&identity).unwrap();
    let object = DatabaseObjectRef::new(
        profile,
        "analytics",
        "public",
        "orders",
        DatabaseObjectKind::Table,
    )
    .unwrap();
    let items = store.knowledge_for_object(&object).await.unwrap();
    assert_eq!(items.len(), 1, "decide wrote no new row: {items:?}");
    assert_eq!(
        items[0].state,
        KnowledgeState::Active,
        "the one matched claim is the one that was confirmed: {items:?}"
    );

    let _ = fs::remove_dir_all(root);
}

/// `decide --profile <name>` reaches a claim in a non-active profile (spec
/// invariant 1b). `review` was not profile-scoped; if `decide` could not reach
/// a claim outside the active profile by name, that would be a real capability
/// loss and the slice must stop. It can, so retiring `review` is safe.
#[tokio::test]
async fn decide_profile_flag_reaches_a_claim_outside_the_active_profile() {
    let root = temp_root("decide_cross_profile");
    // Two profiles; `local` is the configured default (selected via `--profile`),
    // `staging` is a second profile whose claim is NOT reachable from the
    // default. `runtime_at`/`runtime_for_scope` build a fresh runtime with no
    // profile selection, which a two-profile file rejects (`MissingProfile`), so
    // this test builds the runtime inline — mirroring the cross-profile harness
    // in `contracts_slash_parity.rs` — and opens its own store.
    let local_db = root.join("local.sqlite3");
    let staging_db = root.join("staging.sqlite3");
    fs::write(&local_db, b"").unwrap();
    fs::write(&staging_db, b"").unwrap();
    let connections = root.join("connections.toml");
    fs::write(
        &connections,
        format!(
            "[profiles.local]\ntype = 'sqlite'\npath = '{}'\n\n\
             [profiles.staging]\ntype = 'sqlite'\npath = '{}'\n",
            local_db.display(),
            staging_db.display(),
        ),
    )
    .unwrap();
    let options = saya_cli::GlobalOptions {
        connections: Some(connections),
        profile: Some("local".into()),
        ..Default::default()
    };
    let runtime = load_with_sources(&options, &root, &root, BTreeMap::new()).unwrap();
    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    // Migrate the pool and seed an empty cached schema for `staging` so its
    // candidate classifies against a real cache state and the confirm
    // revalidation has a live table to revalidate against.
    let staging_identity = identity_for(&runtime, "staging");
    store
        .upsert_schema(&staging_identity, &orders_schema())
        .await
        .unwrap();

    // Seed a candidate under `staging` only — directly, keyed by staging's
    // identity, so it is invisible to the default (`local`) profile.
    let staging_profile = ProfileIdentity::parse(&staging_identity).unwrap();
    let object = DatabaseObjectRef::new(
        staging_profile,
        "analytics",
        "public",
        "orders",
        DatabaseObjectKind::Table,
    )
    .unwrap();
    let payload = alias_payload();
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
    let staging_id = store
        .knowledge_for_object(&object)
        .await
        .unwrap()
        .into_iter()
        .find(|i| i.slot == KnowledgeSlot::TableAlias)
        .expect("staging candidate seeded")
        .id;
    let staging_id = ClaimId::parse(&staging_id).expect("ki id parses");
    let prefix = staging_id.as_str().chars().take(6).collect::<String>();

    // Without `--profile`, `decide` resolves the default (`local`) and must NOT
    // find the staging claim — proving the claim is genuinely outside the
    // active profile (the thing `review`'s lack of scoping cannot address).
    let (default_code, default_out, default_err) = run(
        ContractsCommand::Decide {
            prefix: prefix.clone(),
            decision: ReviewDecisionArg::Confirm,
            profile: None,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_ne!(
        default_code, 0,
        "decide without --profile must not reach a non-active profile's claim: {default_out}{default_err}"
    );
    let default_combined = format!("{default_out}{default_err}");
    assert!(
        default_combined.contains("no claim matches"),
        "default-profile refusal must say no match: {default_combined}"
    );

    // With `--profile staging`, the staging claim IS reachable and confirmable.
    let (code, out, err) = run(
        ContractsCommand::Decide {
            prefix,
            decision: ReviewDecisionArg::Confirm,
            profile: Some("staging".into()),
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(
        code, 0,
        "decide --profile staging must reach the staging claim: {err}"
    );
    assert!(
        out.contains("confirmed"),
        "staging candidate -> confirmed: {out}"
    );

    // The staging claim alone changed; it is now Active.
    let items = store.knowledge_for_object(&object).await.unwrap();
    assert_eq!(items.len(), 1, "no new row: {items:?}");
    assert_eq!(
        items[0].state,
        KnowledgeState::Active,
        "the staging claim was confirmed: {items:?}"
    );

    let _ = fs::remove_dir_all(root);
}

/// A clap parse with a too-short prefix (`ki-`, two chars) is *accepted* — the
/// floor is enforced at the resolve step, not parse time — but a bare empty
/// prefix is also accepted by clap and refused at resolve. This documents that
/// `decide`'s illegal-input story is "unambiguous or refused" at runtime, the
/// property that retires `review`'s runtime `AmbiguousReview`. (Parse-time
/// refusal of the *decision* value is exercised by the ValueEnum below.)
#[tokio::test]
async fn decide_refuses_an_ambiguous_prefix_at_runtime_not_parse_time() {
    let root = temp_root("decide_ambiguous_prefix");
    let (runtime, _c, _n) = runtime_at(&root);
    let store = store_at(&root).await;
    let _a = seed_candidate(&store, &runtime, "orders").await;
    let _b = seed_candidate(&store, &runtime, "returns").await;

    // `ki-` matches both candidates → ambiguous at the resolve step. clap does
    // not reject it (a two-char prefix parses); the dispatcher refuses.
    let (code, out, err) = run(
        ContractsCommand::Decide {
            prefix: "ki-".into(),
            decision: ReviewDecisionArg::Confirm,
            profile: None,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_ne!(code, 0, "ambiguous prefix must refuse: {out}{err}");
    let combined = format!("{out}{err}");
    assert!(
        combined.contains("more than one claim"),
        "ambiguous refusal names why: {combined}"
    );
    assert!(
        !combined.contains("ki-"),
        "refusal must not echo the prefix: {combined}"
    );

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// S12 — the retired `review` subcommand is gone, and its replacement `decide`
// is what `--help` advertises. `review` was undocumented (never in `docs/`) and
// nothing routes to it but the CLI and a pass-through TUI arm; `decide` covers
// every capability it had (proven above). The illegal `--confirm --reject`
// combination `review` caught at runtime is now unexpressible: `decide` takes a
// single `--decision` ValueEnum clap rejects at parse time.
// ---------------------------------------------------------------------------

/// `saya contracts review …` no longer parses: the subcommand is gone (S12 Q1,
/// outright removal). clap reports an unrecognized subcommand rather than
/// reaching the runtime `AmbiguousReview` path the old `review` had.
#[test]
fn review_subcommand_no_longer_parses() {
    use clap::Parser;
    // `--decision confirm` is `decide`'s vocabulary, not `review`'s; the point
    // is the *subcommand name* `review` is rejected regardless of its args.
    let parsed = Cli::try_parse_from([
        "saya",
        "contracts",
        "review",
        "ki-deadbeef",
        "--confirm",
        "--reject",
    ]);
    assert!(
        parsed.is_err(),
        "`contracts review` must not parse after retirement: {parsed:?}"
    );
    let err = parsed.err().unwrap().to_string();
    assert!(
        !err.contains("choose exactly one"),
        "the runtime AmbiguousReview message must not be reachable: {err}"
    );
}

/// `saya contracts decide …` parses and the illegal input `review` caught at
/// runtime (`--confirm --reject` together) is now unexpressible: `--decision`
/// is a single ValueEnum, so two decisions or an unknown one is a parse error.
#[test]
fn decide_subcommand_parses_and_decision_is_a_single_value_enum() {
    use clap::Parser;
    let parsed = Cli::try_parse_from([
        "saya",
        "contracts",
        "decide",
        "ki-deadbeef",
        "--decision",
        "confirm",
    ]);
    let cli = parsed.expect("`contracts decide` parses");
    let command = cli.command.expect("a subcommand was given");
    let Command::Contracts {
        command: ContractsCommand::Decide { decision, .. },
    } = command
    else {
        panic!("parsed to Decide, got {command:?}");
    };
    assert_eq!(decision, ReviewDecisionArg::Confirm);

    // Two decisions cannot be expressed: clap rejects a repeated `--decision`
    // with a different value.
    let two = Cli::try_parse_from([
        "saya",
        "contracts",
        "decide",
        "ki-deadbeef",
        "--decision",
        "confirm",
        "--decision",
        "reject",
    ]);
    assert!(
        two.is_err(),
        "two --decision values must not parse: {two:?}"
    );

    // An unknown decision value is a parse error, not a runtime path.
    let unknown = Cli::try_parse_from([
        "saya",
        "contracts",
        "decide",
        "ki-deadbeef",
        "--decision",
        "maybe",
    ]);
    assert!(
        unknown.is_err(),
        "an unknown --decision must not parse: {unknown:?}"
    );
}

/// `saya contracts --help` advertises `decide` and not `review`: the retired
/// command left the help surface, and its replacement is what a user finds.
#[test]
fn contracts_help_advertises_decide_not_review() {
    use clap::CommandFactory;
    // Render the help for the `contracts` subcommand. `find_subcommand`
    // returns a `&Command` but `render_help` takes `&mut self`, so clone the
    // subcommand. `render_help` returns a `StyledStr` (clap 4.6) that
    // stringifies to the plain help text.
    let help = Cli::command()
        .find_subcommand("contracts")
        .expect("contracts subcommand exists")
        .clone()
        .render_help()
        .to_string();
    assert!(
        help.contains("decide"),
        "contracts --help must advertise decide: {help}"
    );
    // `review` appears in other subcommand prose (e.g. `queue`'s "pending
    // review"), so assert on the subcommand *entry* line, not a bare substring.
    assert!(
        !help
            .lines()
            .any(|line| line.trim_start().starts_with("review ")),
        "contracts --help must not list a `review` subcommand: {help}"
    );
}
