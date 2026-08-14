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
use saya_store::{ContractStore, ProposeClaim, ProposeOutcome, SchemaStore, SqliteStateStore};
use saya_types::{
    ClaimId, ClaimOrigin, ClaimPayload, ClaimStatus, DatabaseObjectKind, DatabaseObjectRef,
    ProfileIdentity, SchemaFingerprint, SchemaTree,
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

    let cand_id = seed_candidate(&store, &runtime, "orders").await;
    let queue = ContractsCommand::Queue {
        profile: None,
        limit: None,
    };
    let (code, out, err) = run(queue, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "queue stderr: {err}");
    // The queue is a worklist: every entry is a candidate by definition, so the
    // status word is not rendered. The fields a reviewer needs are the id, the
    // kind/value, the object, the schema state, and the evidence count.
    assert!(
        out.contains(cand_id.as_str()),
        "queue must name the candidate's full id: {out}"
    );
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
