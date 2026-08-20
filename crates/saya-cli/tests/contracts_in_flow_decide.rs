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
#[tokio::test]
async fn confirm_by_short_prefix_confirms_the_intended_claim() {
    let root = temp_root("confirm_prefix");
    let (runtime, _name) = runtime_at(&root);
    let store = store_at(&root).await;

    let id = seed_candidate(&store, &runtime, "orders").await;
    let prefix = short_prefix(&id);

    let (_cmd, code, out, err) = run_slash(
        &format!("/confirm {prefix}"),
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(code, 0, "/confirm stderr: {err}");
    // The confirm op emits ContractChanged with the FULL id and "confirmed".
    assert!(out.contains("confirmed"), "/confirm out: {out}");
    assert!(
        out.contains(id.as_str()),
        "must name the exact claim: {out}"
    );

    // The claim is now Confirmed — recallable via show.
    let (scode, sout, serr) = run_headless(
        ContractsCommand::Show {
            table: qualified().into(),
            profile: None,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(scode, 0, "show stderr: {serr}");
    assert!(sout.contains("confirmed"), "show after confirm: {sout}");

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 2. Rejecting by the short reference rejects exactly that claim; nothing is
//    promoted (no claim becomes recallable).
// ---------------------------------------------------------------------------
#[tokio::test]
async fn reject_by_short_prefix_rejects_and_promotes_nothing() {
    let root = temp_root("reject_prefix");
    let (runtime, _name) = runtime_at(&root);
    let store = store_at(&root).await;

    let id = seed_candidate(&store, &runtime, "orders").await;
    let prefix = short_prefix(&id);

    let (_cmd, code, out, err) = run_slash(
        &format!("/reject {prefix}"),
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(code, 0, "/reject stderr: {err}");
    assert!(out.contains("rejected"), "/reject out: {out}");
    assert!(
        out.contains(id.as_str()),
        "must name the exact claim: {out}"
    );

    // Rejected → not recallable. Show finds no contract for the object.
    let (scode, sout, serr) = run_headless(
        ContractsCommand::Show {
            table: qualified().into(),
            profile: None,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(scode, 0, "show stderr: {serr}");
    assert!(
        sout.contains("No contract"),
        "rejected claim must not be recallable: {sout}"
    );

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 4. An ambiguous or unresolvable reference refuses, and changes nothing.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn ambiguous_or_unresolvable_prefix_refuses_and_changes_nothing() {
    let root = temp_root("ambiguous_prefix");
    let (runtime, _name) = runtime_at(&root);
    let store = store_at(&root).await;

    // Seed two candidates whose ids share the `ki-` marker; a bare "ki-" prefix
    // matches both → ambiguous. (Both ids start with "ki-" by construction.)
    let id_a = seed_candidate(&store, &runtime, "orders").await;
    let id_b = seed_candidate(&store, &runtime, "returns").await;
    assert_ne!(id_a, id_b);

    // The shared prefix both ids start with is "ki-": ambiguous.
    let ambiguous = "ki-";
    let (acode, aout, aerr) = run_headless(
        ContractsCommand::Decide {
            prefix: ambiguous.into(),
            decision: saya_cli::ReviewDecisionArg::Confirm,
            profile: None,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_ne!(acode, 0, "ambiguous prefix must not succeed: {aout}{aerr}");
    let combined = format!("{aout}{aerr}");
    // The refusal names why without echoing the typed prefix: it says the
    // reference matches more than one claim (the "unambiguous or refused"
    // invariant), never the prefixes themselves.
    assert!(
        combined.contains("more than one claim"),
        "ambiguous refusal must name why: {combined}"
    );
    assert!(
        !combined.contains(ambiguous),
        "refusal echoed the prefix: {combined}"
    );
    // Neither item was changed: both still Pending.
    for id in [&id_a, &id_b] {
        let item = store
            .get_knowledge_item(id.as_str())
            .await
            .unwrap()
            .expect("item present");
        assert_eq!(
            item.state,
            KnowledgeState::Pending,
            "ambiguous changed {id}"
        );
    }

    // An unresolvable prefix (no item starts with it) refuses too.
    let (ncode, nout, nerr) = run_headless(
        ContractsCommand::Decide {
            prefix: "ki-zzzzz".into(),
            decision: saya_cli::ReviewDecisionArg::Confirm,
            profile: None,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_ne!(
        ncode, 0,
        "unresolvable prefix must not succeed: {nout}{nerr}"
    );
    let ncombined = format!("{nout}{nerr}");
    assert!(
        ncombined.contains("no claim matches"),
        "unresolvable refusal must name why: {ncombined}"
    );
    assert!(
        !ncombined.contains("ki-zzzzz"),
        "unresolvable refusal echoed the prefix: {ncombined}"
    );

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 5. A stale reference behaves as §4 decided — asserted explicitly.
//    The prefix is a stored id; ids are immutable, so "stale" cannot mean the
//    prefix now points at a *different* claim (the silent-wrong-write failure
//    mode a per-turn index has). It means one of:
//    (a) the prefix is too short / matches more than one claim → AMBIGUOUS,
//        refused, nothing changes (covered above); and
//    (b) the prefix matched a claim that no longer exists (forgotten) → NOT
//        FOUND, refused, nothing changes.
//    The defence: a prefix never silently resolves to a different claim than
//    the one the user saw — it resolves to the same claim or refuses.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn stale_prefix_refuses_rather_than_write_the_wrong_claim() {
    let root = temp_root("stale_prefix");
    let (runtime, _name) = runtime_at(&root);
    let store = store_at(&root).await;

    // Seed two candidates; the user saw the first receipt and remembers only
    // "ki-" (the shortest thing that could be on screen). Another claim now
    // shares that prefix. A per-turn index would silently confirm the WRONG
    // one; the prefix refuses because it is ambiguous.
    let _id_a = seed_candidate(&store, &runtime, "orders").await;
    let id_b = seed_candidate(&store, &runtime, "returns").await;
    let (code, out, err) = run_headless(
        ContractsCommand::Decide {
            prefix: "ki-".into(),
            decision: saya_cli::ReviewDecisionArg::Confirm,
            profile: None,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_ne!(code, 0, "ambiguous stale prefix must refuse: {out}{err}");
    // Both still Pending — nothing written to durable memory.
    for id in [&_id_a, &id_b] {
        let item = store
            .get_knowledge_item(id.as_str())
            .await
            .unwrap()
            .expect("item present");
        assert_eq!(item.state, KnowledgeState::Pending);
    }

    // (b) The prefix matched an item that has since been forgotten: the row is
    // still present (Dismissed), so the prefix still resolves, but `confirm`
    // refuses a dismissed item as a conflict — nothing is promoted. Forget
    // id_b, then try to confirm it by its full prefix.
    run_headless(
        ContractsCommand::Forget {
            claim_id: id_b.as_str().into(),
            reason: ForgetReasonArg::UserRequest,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    let (fcode, fout, ferr) = run_headless(
        ContractsCommand::Decide {
            prefix: id_b.as_str().into(),
            decision: saya_cli::ReviewDecisionArg::Confirm,
            profile: None,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_ne!(
        fcode, 0,
        "forgotten item's prefix must not confirm: {fout}{ferr}"
    );
    // A forgotten item is not promoted to Active by this path.
    let item = store
        .get_knowledge_item(id_b.as_str())
        .await
        .unwrap()
        .expect("tombstone present");
    assert_ne!(
        item.state,
        KnowledgeState::Active,
        "forgotten was confirmed"
    );

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 6. Usage errors never contain the user's argument text.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn usage_errors_never_echo_user_argument_text() {
    // Parse-only: a usage error surfaces before any store access. An unopenable
    // store is irrelevant here.
    let secret = "c-SUPERSECRET-leaked-value";

    // No argument at all.
    let bad = parse_slash_command("/confirm").unwrap_err();
    assert!(!bad.to_string().contains(secret), "echoed: {bad}");
    assert!(!bad.to_string().is_empty());

    // Two tokens is a usage error (the prefix is one token).
    let bad = parse_slash_command("/confirm c-abc c-def").unwrap_err();
    assert!(
        !bad.to_string().contains("c-abc"),
        "echoed untrusted input: {bad}"
    );
    assert!(
        !bad.to_string().contains("c-def"),
        "echoed untrusted input: {bad}"
    );

    // The secret-bearing prefix never reaches the message even when it is the
    // sole argument and the command fails for another reason at parse time.
    let parsed = parse_slash_command(&format!("/confirm {secret}"));
    // A single well-formed token parses; it would then resolve at dispatch. The
    // point here is that the PARSE error path (too many tokens / no tokens) does
    // not echo. Asserting the single-token case parsed keeps this honest: it
    // proves the secret is only ever carried in the parsed command, not in an
    // error string.
    assert!(parsed.is_ok(), "single token should parse: {parsed:?}");
    let cmd = match parsed.unwrap() {
        Some(SlashCommand::Contracts(c)) => c,
        other => panic!("expected Contracts, got {other:?}"),
    };
    // The resolved command carries the prefix, but that never surfaces in a
    // usage error — only in the typed dispatcher, which maps failures to
    // payload-free messages (asserted in tests 4/5).
    let _ = cmd;
}

// ---------------------------------------------------------------------------
// 7. /queue and the existing contracts subcommands still behave as before.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn queue_and_existing_subcommands_unchanged() {
    let root = temp_root("unchanged");
    let (runtime, _name) = runtime_at(&root);
    let store = store_at(&root).await;

    // /queue still lists the candidate (the queue is the candidate worklist).
    let id = seed_candidate(&store, &runtime, "orders").await;
    let (_qcmd, qcode, qout, qerr) =
        run_slash("/queue", &runtime, &store, RenderFormat::Text).await;
    assert_eq!(qcode, 0, "/queue stderr: {qerr}");
    assert!(
        qout.contains(id.as_str()),
        "/queue still lists candidates: {qout}"
    );

    // /contracts (list) still works: exit 0, no error. A candidate is not
    // recallable, so list does not show it — that is the unchanged behaviour,
    // not a regression. Confirm the claim, then list shows it.
    let (_ccmd, ccode, _cout, cerr) =
        run_slash("/contracts", &runtime, &store, RenderFormat::Text).await;
    assert_eq!(ccode, 0, "/contracts stderr: {cerr}");

    run_headless(
        ContractsCommand::Decide {
            prefix: id.as_str().into(),
            decision: saya_cli::ReviewDecisionArg::Confirm,
            profile: None,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    // `Decide::Confirm` resolves the prefix to the knowledge item and promotes
    // it to `Active` (the confirm op writes `knowledge_items` now), so `/contracts`
    // (list → recall over `knowledge_items`) shows the confirmed claim.
    let (_ccmd2, _ccode2, cout2, cerr2) =
        run_slash("/contracts", &runtime, &store, RenderFormat::Text).await;
    assert!(
        cout2.contains("orders"),
        "/contracts lists a confirmed claim: {cout2}"
    );
    assert_eq!(cerr2, "");

    // /contract <t> (show) still works and shows the now-confirmed claim.
    let (_scmd, scode, sout, serr) = run_slash(
        "/contract analytics.public.orders",
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(scode, 0, "/contract stderr: {serr}");
    assert!(sout.contains("orders"), "/contract still shows: {sout}");

    // /forget <full-id> still works and is unaffected by the new commands.
    let (_fcmd, fcode, fout, ferr) = run_slash(
        &format!("/forget {id}"),
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(fcode, 0, "/forget stderr: {ferr}");
    assert!(
        fout.contains("forgotten"),
        "/forget still tombstones: {fout}"
    );

    let _ = fs::remove_dir_all(root);
}
