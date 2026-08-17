//! Headless `saya contracts` commands: drives the command adapter directly
//! (constructs `ContractsCommand` values and calls `run_contracts`), never
//! shelling out to a built binary. Covers the spec at
//! `.claude/specs/spec-2b2c-contract-commands.md`.
//!
//! Output is captured through the thread-local seam in `output::emit` so the
//! tests can assert on rendered text/JSON/NDJSON without touching the process
//! stdout (which would race under parallel test runs).

use saya_cli::{
    ClaimKindArg, ContractsCommand, ForgetReasonArg, RenderFormat, RuntimeConfig,
    capture_output_start, capture_output_take, load_with_sources, profile_identity, run_contracts,
};
use saya_store::{
    ContractStore, KnowledgeItemRequest, KnowledgeItemStore, ProposeClaim, ProposeOutcome,
    SchemaStore, SqliteStateStore,
};
use saya_types::{
    ClaimId, ClaimOrigin, ClaimPayload, ClaimStatus, Column, Database, DatabaseObjectKind,
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

/// Propose a candidate claim directly through the store, so `review --confirm`
/// has something that is *not* already confirmed to act on. `remember` only
/// stores confirmed claims, so candidates must be seeded out-of-band.
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
    let fingerprint = unobserved_fingerprint();
    let request = ProposeClaim {
        object: object.clone(),
        fingerprint,
        payload: alias_payload(),
        origin: ClaimOrigin::AssistantInferred,
        initial_status: ClaimStatus::Candidate,
        evidence: None,
        referenced_columns: Vec::new(),
    };
    match store.propose_claim(request).await.unwrap() {
        ProposeOutcome::Stored(id) => id,
        other => panic!("expected Stored, got {other:?}"),
    }
}

/// The "no schema observed" fingerprint the headless adapter stores: current
/// format, all-zero digest, guaranteed never to equal a real schema's digest so
/// a later live schema never reads the claim as `current`.
fn unobserved_fingerprint() -> SchemaFingerprint {
    SchemaFingerprint::from_parts(saya_types::FINGERPRINT_VERSION, &"0".repeat(64)).unwrap()
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
// 2. remember twice -> duplicate with the same id, no second claim
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
        profile: None,
    };
    let (code1, out1, err1) = run(remember.clone(), &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code1, 0, "stderr: {err1}");
    let first_id = out1
        .strip_prefix("remembered ")
        .and_then(|rest| rest.split_whitespace().next())
        .expect("first line names the id: {out1}");

    let (code2, out2, err2) = run(remember, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code2, 0, "stderr: {err2}");
    assert!(
        out2.contains("duplicate"),
        "second must be duplicate: {out2}"
    );
    assert!(
        out2.contains(first_id),
        "duplicate must echo the same id {first_id}: {out2}"
    );

    // Exactly one claim exists for the object.
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
    let claims = store.list_claims(&object, &[]).await.unwrap();
    assert_eq!(
        claims.len(),
        1,
        "duplicate created a second claim: {claims:?}"
    );

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
        profile: None,
    };
    let (code, out, err) = run(remember, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "stderr: {err}");
    let id = out
        .strip_prefix("remembered ")
        .and_then(|rest| rest.split_whitespace().next())
        .expect("id present: {out}");

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
        profile: None,
    };
    let (code, out, err) = run(remember.clone(), &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "stderr: {err}");
    let id = out
        .strip_prefix("remembered ")
        .and_then(|rest| rest.split_whitespace().next())
        .expect("id present: {out}");

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
        out.contains("forgotten"),
        "duplicate of a forgotten claim must say forgotten: {out}"
    );
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
    let claims = store.list_claims(&object, &[]).await.unwrap();
    assert_eq!(
        claims.len(),
        1,
        "re-remember created a new claim: {claims:?}"
    );

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 5. review --confirm: candidate -> confirmed; already-confirmed -> typed conflict
// ---------------------------------------------------------------------------
#[tokio::test]
async fn review_confirm_on_candidate_confirms_and_on_confirmed_conflicts() {
    let root = temp_root("review");
    let (runtime, _c, _n) = runtime_at(&root);
    let store = store_at(&root).await;

    let candidate_id = seed_candidate(&store, &runtime, "orders").await;

    let confirm = ContractsCommand::Review {
        claim_id: candidate_id.as_str().into(),
        confirm: true,
        reject: false,
    };
    let (code, out, err) = run(confirm, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "stderr: {err}");
    assert!(out.contains("confirmed"), "candidate -> confirmed: {out}");

    // Confirming an already-confirmed claim is a typed conflict, not success.
    let (code, out, err) = run(
        ContractsCommand::Review {
            claim_id: candidate_id.as_str().into(),
            confirm: true,
            reject: false,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_ne!(code, 0, "already-confirmed must not succeed: {out}{err}");
    let combined = format!("{out}{err}");
    assert!(
        combined.contains("conflict"),
        "expected a typed conflict, got: {combined}"
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
        profile: None,
    };
    // The text `remember` line carries the full claim id the user pastes.
    let (code, text_out, err) = run(remember.clone(), &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "stderr: {err}");
    let text_id = text_out
        .strip_prefix("remembered ")
        .and_then(|rest| rest.split_whitespace().next())
        .expect("text form names the id: {text_out}");

    // The duplicate event (same value remembered again) carries the same id in
    // JSON and NDJSON; it parses and agrees with the text id.
    for format in [RenderFormat::Json, RenderFormat::Ndjson] {
        let (code, out, err) = run(remember.clone(), &runtime, &store, format).await;
        assert_eq!(code, 0, "{format:?} stderr: {err}");
        let events = json_events(&out);
        let changed = events
            .iter()
            .find(|v| v["event"] == "contract_changed")
            .expect("a contract_changed event was emitted");
        assert_eq!(changed["action"], "duplicate");
        assert_eq!(
            changed["status"], "confirmed",
            "duplicate of a confirmed claim carries its status"
        );
        let id = changed["claim_id"].as_str().expect("claim_id is a string");
        assert_eq!(id, text_id, "{format:?} duplicate id disagrees with text");
    }

    // `show` carries the same claim id in every format, including text.
    let show = ContractsCommand::Show {
        table: qualified().into(),
        profile: None,
    };
    // The text show renders the abbreviated id; the full id still appears in the
    // JSON/NDJSON contract_show event, so compare against those.
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
        assert_eq!(claim_id, text_id, "{format:?} show id disagrees with text");
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
//     queue reuses the existing `review` operation — no new write tool.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn queue_lists_candidates_and_review_transitions_them() {
    let root = temp_root("queue_review");
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
    // the queue on the next read.
    let confirm = ContractsCommand::Review {
        claim_id: cand_id.as_str().into(),
        confirm: true,
        reject: false,
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
    let reject = ContractsCommand::Review {
        claim_id: cand_id.as_str().into(),
        confirm: false,
        reject: true,
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

/// Seeds a confirmed claim whose stored fingerprint *equals* the cached
/// table's, so a matching cache classifies it `current`. `remember` would
/// store the unobserved all-zeros sentinel, which never equals a real digest
/// and so could only ever read `needs_review` against a live table — not
/// enough to prove the bug is fixed.
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
    let fingerprint = SchemaFingerprint::of_table(DatabaseObjectKind::Table, &orders_table());
    let payload = ClaimPayload::table_alias("orders").unwrap();
    // The legacy claim populates `contract_objects` so `contracts list`'s
    // `list_objects` (still legacy until the CLI chunk migrates it) returns the
    // object as an explicit ref. The D-3 knowledge item is what `recall` — which
    // `list` calls — now reads, so seed both: the legacy row for the object
    // list, the knowledge item for the contract recall supplies.
    let request = ProposeClaim {
        object: object.clone(),
        fingerprint,
        payload: payload.clone(),
        origin: ClaimOrigin::UserExplicit,
        initial_status: ClaimStatus::Confirmed,
        evidence: None,
        referenced_columns: Vec::new(),
    };
    match store.propose_claim(request).await.unwrap() {
        ProposeOutcome::Stored(_) => {}
        other => panic!("expected Stored, got {other:?}"),
    }
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
        profile: None,
    };
    let (code, out, err) = run(remember, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "remember stderr: {err}");
    assert!(out.contains("remembered"), "out: {out}");

    // The stored claim's fingerprint equals the cached table's real digest,
    // not the all-zeros sentinel. This is the load-bearing assertion: it is
    // what makes the claim read `current` instead of `needs_review`.
    let profile = ProfileIdentity::parse(&identity).unwrap();
    let object = DatabaseObjectRef::new(
        profile,
        "analytics",
        "public",
        "orders",
        DatabaseObjectKind::Table,
    )
    .unwrap();
    let claims = store.list_claims(&object, &[]).await.unwrap();
    assert_eq!(claims.len(), 1, "expected one claim: {claims:?}");
    let real = SchemaFingerprint::of_table(DatabaseObjectKind::Table, &orders_table());
    assert_eq!(
        claims[0].schema_fingerprint, real,
        "remember stored the unobserved sentinel, not the cached table's real digest"
    );

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

// A column-level claim remembered against a cached schema must store a TYPED
// column snapshot (resolved type + nullability), so a later schema change can
// tell a retyped referenced column from an unrelated one. Before the fix the
// name-only snapshot (empty type) made every column claim read needs_review.
#[tokio::test]
async fn remember_column_claim_against_cached_schema_snapshots_real_type() {
    let root = temp_root("remember_column_snapshot_real_type");
    let (runtime, _c, _n) = runtime_at(&root);
    let store = store_at(&root).await;
    let identity = identity_for(&runtime, "local");
    // orders table with a typed `id` column the claim references.
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
    let claims = store.list_claims(&object, &[]).await.unwrap();
    assert_eq!(claims.len(), 1, "{claims:?}");
    // The snapshot carries the resolved type, not the empty type of the
    // name-only sentinel path.
    assert_eq!(claims[0].referenced_columns.len(), 1, "{claims:?}");
    assert_eq!(claims[0].referenced_columns[0].name, "id");
    assert_eq!(
        claims[0].referenced_columns[0].data_type, "bigint",
        "column snapshot must carry the cached type, not be empty: {:?}",
        claims[0].referenced_columns
    );
    assert!(!claims[0].referenced_columns[0].nullable);

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
    let claims = store.list_claims(&ghost, &[]).await.unwrap();
    assert!(
        claims.is_empty(),
        "refuse must store nothing for the unknown object: {claims:?}"
    );

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 15. No-cache guard: with NO cached schema `remember` keeps today's behaviour
//     — it succeeds and stores the unobserved sentinel, and `show` reads
//     `live_schema_unavailable`. Refusing would make remember unusable before a
//     first refresh, so the fix must not touch this path.
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
    let claims = store.list_claims(&object, &[]).await.unwrap();
    assert_eq!(claims.len(), 1, "{claims:?}");
    assert_eq!(
        claims[0].schema_fingerprint,
        unobserved_fingerprint(),
        "no-cache remember must store the unobserved sentinel, not a real digest"
    );

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
    let fingerprint = SchemaFingerprint::of_table(DatabaseObjectKind::Table, &orders_table());
    let request = ProposeClaim {
        object: object.clone(),
        fingerprint,
        payload: ClaimPayload::table_alias("orders").unwrap(),
        origin: ClaimOrigin::AssistantInferred,
        initial_status: ClaimStatus::Candidate,
        evidence: None,
        referenced_columns: Vec::new(),
    };
    match store.propose_claim(request).await.unwrap() {
        ProposeOutcome::Stored(id) => id,
        other => panic!("expected Stored, got {other:?}"),
    }
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
// 17. ITEM 2 regression: the queue lists both `Candidate` and persisted
//     `Stale` claims (since 5d), and a reviewer cannot tell which decision is
//     being asked unless the status is rendered. Both appear, and each line
//     carries its own status word — `candidate` for a fresh claim to confirm
//     or reject, `stale` for a confirmed claim reconciliation marked because
//     the schema drifted. The two are distinguishable in the output.
// ---------------------------------------------------------------------------
async fn seed_stale_claim(
    store: &SqliteStateStore,
    runtime: &RuntimeConfig,
    table: &str,
) -> ClaimId {
    // A claim reaches `Stale` through `mark_stale` (reconcile), never through
    // `propose_claim`, which only admits `Candidate`/`Confirmed`. Seed a
    // candidate and transition it.
    let id = seed_candidate(store, runtime, table).await;
    store.mark_stale(&id).await.expect("candidate -> stale");
    id
}

#[tokio::test]
async fn queue_distinguishes_candidate_from_stale_in_output() {
    let root = temp_root("queue_candidate_vs_stale");
    let (runtime, _c, _n) = runtime_at(&root);
    let store = store_at(&root).await;
    // Drop `store_at`'s empty default cache so neither claim has a cached
    // schema and both read `live_schema_unavailable`. That keeps the only
    // `candidate`/`stale` words on each line the status tokens themselves —
    // the distinction under test — rather than also a `[stale]` schema state.
    let identity = identity_for(&runtime, "local");
    store.invalidate_schema(&identity).await.unwrap();

    let cand_id = seed_candidate(&store, &runtime, "orders").await;
    // A second object so the stale claim does not dedup against the candidate.
    let stale_id = seed_stale_claim(&store, &runtime, "shipments").await;

    let queue = ContractsCommand::Queue {
        profile: None,
        limit: None,
    };
    let (code, out, err) = run(queue.clone(), &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "queue stderr: {err}");

    // Both claims appear, each on its own line.
    assert!(
        out.contains(cand_id.as_str()),
        "candidate id missing: {out}"
    );
    assert!(out.contains(stale_id.as_str()), "stale id missing: {out}");

    // The two status words are distinct and both render.
    assert!(
        out.contains("candidate"),
        "queue must show candidate: {out}"
    );
    assert!(out.contains("stale"), "queue must show stale: {out}");

    // Each line carries its own status word as its second whitespace-delimited
    // token (right after the claim id), distinct from the `[stale]` schema
    // state that may appear later on the same line. Assert on the token so the
    // status and the schema state — which can both be `stale` — are not
    // conflated.
    fn status_token(line: &str) -> &str {
        line.split_whitespace().nth(1).unwrap_or("")
    }
    let cand_line = out
        .lines()
        .find(|line| line.contains(cand_id.as_str()))
        .expect("candidate line present");
    assert_eq!(
        status_token(cand_line),
        "candidate",
        "candidate line's status token must be candidate: {cand_line}"
    );
    let stale_line = out
        .lines()
        .find(|line| line.contains(stale_id.as_str()))
        .expect("stale line present");
    assert_eq!(
        status_token(stale_line),
        "stale",
        "stale line's status token must be stale: {stale_line}"
    );

    // The JSON form carries the status field too, so machine readers distinguish
    // the two the same way.
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
    let stale_item = items
        .iter()
        .find(|v| v["claim_id"] == stale_id.as_str())
        .expect("stale item present in JSON");
    assert_eq!(
        stale_item["status"], "stale",
        "JSON stale status: {stale_item}"
    );

    let _ = fs::remove_dir_all(root);
}
