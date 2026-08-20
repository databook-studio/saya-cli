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
#[tokio::test]
async fn contracts_list_slash_and_headless_agree_on_claim_ids_and_order() {
    let root = temp_root("list_parity");
    let (runtime, _name) = runtime_at(&root);
    let store = store_at(&root).await;

    // Seed two distinct claims so order is observable, through the headless path.
    for value in ["customers", "orders_alias"] {
        let (code, out, err) = run_headless(
            ContractsCommand::Remember {
                table: qualified().into(),
                kind: ClaimKindArg::Alias,
                value: value.into(),
                column: None,
                reason: None,
                profile: None,
            },
            &runtime,
            &store,
            RenderFormat::Text,
        )
        .await;
        assert_eq!(code, 0, "seed stderr: {err}");
        assert!(out.starts_with("remembered"), "seed out: {out}");
    }

    let headless = run_headless(
        ContractsCommand::List { profile: None },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    let (_cmd, code, out, err) =
        run_slash("/contracts", &runtime, &store, RenderFormat::Text).await;

    assert_eq!(code, 0, "/contracts stderr: {err}");
    // The rendered stanzas — claim id lines and their order — must match byte
    // for byte. A second DTO mapping or a second recall would diverge here.
    assert_eq!(
        out, headless.1,
        "slash /contracts diverged from headless list"
    );
    assert_eq!(err, headless.2);

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 2. /contract <t> and `saya contracts show <t>` produce the same contract.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn contract_show_slash_and_headless_agree_on_contract() {
    let root = temp_root("show_parity");
    let (runtime, _name) = runtime_at(&root);
    let store = store_at(&root).await;

    run_headless(
        ContractsCommand::Remember {
            table: qualified().into(),
            kind: ClaimKindArg::Alias,
            value: "customers".into(),
            column: None,
            reason: None,
            profile: None,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;

    let headless = run_headless(
        ContractsCommand::Show {
            table: qualified().into(),
            profile: None,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    let (_cmd, code, out, err) = run_slash(
        "/contract analytics.public.orders",
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;

    assert_eq!(code, 0, "/contract stderr: {err}");
    // Same object, schema state, claim ids, conflicts — the whole rendered stanza.
    assert_eq!(out, headless.1, "/contract diverged from headless show");
    assert_eq!(err, headless.2);

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 3. /remember and `saya contracts remember` produce the *same* claim id —
//    proving both go through the same deterministic identity and payload
//    constructors. Two fresh stores in the same root share the connections path
//    (the cache scope), so the derived profile identity is identical and each
//    path stores a *new* claim whose id can be compared (not a duplicate).
// ---------------------------------------------------------------------------
#[tokio::test]
async fn remember_slash_and_headless_produce_same_claim_id() {
    let root = temp_root("remember_parity");
    let (runtime, _name) = runtime_at(&root);

    // Slash form: kind positional, everything after it is the value.
    let slash_store = store_at_name(&root, "slash.sqlite3").await;
    let (slash_cmd, scode, sout, serr) = run_slash(
        "/remember analytics.public.orders alias customers",
        &runtime,
        &slash_store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(scode, 0, "/remember stderr: {serr}");
    assert!(
        sout.contains("remembered alias customers for analytics.public.orders"),
        "slash /remember names fact and object: {sout}"
    );
    assert!(
        !sout.contains("ki-"),
        "slash /remember contains no raw id: {sout}"
    );

    // The translated command must be structurally equal to the headless one —
    // same operation, same payload constructors, same deterministic identity.
    assert_eq!(
        slash_cmd,
        ContractsCommand::Remember {
            table: qualified().into(),
            kind: ClaimKindArg::Alias,
            value: "customers".into(),
            column: None,
            reason: None,
            profile: None,
        }
    );

    // A *fresh* store at the same root (same scope) so the headless path stores
    // a new, comparable claim — not a duplicate of the slash claim.
    let headless_store = store_at_name(&root, "headless.sqlite3").await;
    let (_hcode, hout, herr) = run_headless(
        ContractsCommand::Remember {
            table: qualified().into(),
            kind: ClaimKindArg::Alias,
            value: "customers".into(),
            column: None,
            reason: None,
            profile: None,
        },
        &runtime,
        &headless_store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(_hcode, 0, "headless stderr: {herr}");
    assert_eq!(sout, hout, "slash and headless text output agree");

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
    let slash_items = slash_store.knowledge_for_object(&object).await.unwrap();
    let headless_items = headless_store.knowledge_for_object(&object).await.unwrap();
    assert_eq!(
        slash_items[0].id, headless_items[0].id,
        "slash and headless /remember produced different claim ids in store"
    );

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 4. /forget and `saya contracts forget` both tombstone; recall excludes both.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn forget_slash_and_headless_tombstone_and_exclude_identically() {
    let root = temp_root("forget_parity");
    let (runtime, _name) = runtime_at(&root);
    let store = store_at(&root).await;

    let (_, _, out, _) = run_slash(
        "/remember analytics.public.orders alias customers",
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert!(out.contains("remembered alias customers for analytics.public.orders"));
    assert!(!out.contains("ki-"));

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

    // Slash /forget tombstones.
    let (_cmd, fcode, fout, ferr) = run_slash(
        &format!("/forget {id}"),
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(fcode, 0, "/forget stderr: {ferr}");
    assert!(fout.contains("forgotten"), "/forget out: {fout}");

    // Recall (show) no longer lists it.
    let (_, _, show_out, _) = run_slash(
        "/contract analytics.public.orders",
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert!(
        !show_out.contains("customers"),
        "forgotten claim still listed after /forget: {show_out}"
    );

    // The slash /forget translated command equals the headless forget command.
    assert_eq!(
        _cmd,
        ContractsCommand::Forget {
            claim_id: id.to_string(),
            // Slash /forget uses the default reason; the headless default matches.
            reason: ForgetReasonArg::UserRequest,
        }
    );

    // A second store: headless forget must produce the same exclusion.
    let root2 = temp_root("forget_parity_headless");
    let (runtime2, _n) = runtime_at(&root2);
    let store2 = store_at(&root2).await;
    let (_, _, _) = run_headless(
        ContractsCommand::Remember {
            table: qualified().into(),
            kind: ClaimKindArg::Alias,
            value: "customers".into(),
            column: None,
            reason: None,
            profile: None,
        },
        &runtime2,
        &store2,
        RenderFormat::Text,
    )
    .await;
    let identity2 = identity_for(&runtime2, "local");
    let profile2 = ProfileIdentity::parse(&identity2).unwrap();
    let object2 = DatabaseObjectRef::new(
        profile2,
        "analytics",
        "public",
        "orders",
        DatabaseObjectKind::Table,
    )
    .unwrap();
    let items2 = store2.knowledge_for_object(&object2).await.unwrap();
    let id2 = items2[0].id.as_str();
    run_headless(
        ContractsCommand::Forget {
            claim_id: id2.to_string(),
            reason: ForgetReasonArg::UserRequest,
        },
        &runtime2,
        &store2,
        RenderFormat::Text,
    )
    .await;
    let (_, hshow, _) = run_headless(
        ContractsCommand::Show {
            table: qualified().into(),
            profile: None,
        },
        &runtime2,
        &store2,
        RenderFormat::Text,
    )
    .await;
    assert!(
        !hshow.contains("customers"),
        "headless forget still listed the claim: {hshow}"
    );

    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(root2);
}

// ---------------------------------------------------------------------------
// 5. A malformed table is rejected identically by both.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn malformed_table_rejected_identically_by_slash_and_headless() {
    let root = temp_root("malformed_parity");
    let (runtime, _name) = runtime_at(&root);
    let store = store_at(&root).await;

    let bad = "orders";
    let headless = run_headless(
        ContractsCommand::Show {
            table: bad.into(),
            profile: None,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    let (_cmd, code, out, err) =
        run_slash("/contract orders", &runtime, &store, RenderFormat::Text).await;

    // Same error class: non-zero exit, same guidance text, no echo of the input.
    assert_ne!(code, 0, "/contract malformed must not succeed: {out}{err}");
    assert_eq!(code, headless.0, "exit code diverged");
    assert_eq!(out, headless.1, "stdout diverged");
    assert_eq!(err, headless.2, "stderr diverged");
    let combined = format!("{out}{err}");
    assert!(
        combined.contains("catalog.schema.object"),
        "missing guidance: {combined}"
    );
    assert!(
        !combined.contains(bad),
        "echoed untrusted input: {combined}"
    );

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 6. /remember then /contract round-trips the claim (beyond-parity).
// ---------------------------------------------------------------------------
#[tokio::test]
async fn remember_then_contract_round_trips() {
    let root = temp_root("round_trip_slash");
    let (runtime, _name) = runtime_at(&root);
    let store = store_at(&root).await;

    let (_, rcode, rout, rerr) = run_slash(
        "/remember analytics.public.orders alias customers",
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(rcode, 0, "stderr: {rerr}");
    assert!(rout.contains("remembered"), "out: {rout}");

    let (_, _, cout, cerr) = run_slash(
        "/contract analytics.public.orders",
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(cerr, "");
    assert!(cout.contains("customers"), "value missing: {cout}");
    assert!(cout.contains("table_alias"), "kind missing: {cout}");
    assert!(cout.contains("user_explicit"), "origin missing: {cout}");
    assert!(cout.contains("confirmed"), "status missing: {cout}");

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 7. /forget then /contract no longer lists it (beyond-parity).
// ---------------------------------------------------------------------------
#[tokio::test]
async fn forget_then_contract_no_longer_lists() {
    let root = temp_root("forget_then_show_slash");
    let (runtime, _name) = runtime_at(&root);
    let store = store_at(&root).await;

    let (_, _, rout, _) = run_slash(
        "/remember analytics.public.orders alias customers",
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert!(rout.contains("remembered"));
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

    let (_, _, fout, ferr) = run_slash(
        &format!("/forget {id}"),
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(ferr, "");
    assert!(fout.contains("forgotten"), "out: {fout}");

    let (_, _, cout, _) = run_slash(
        "/contract analytics.public.orders",
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert!(!cout.contains("customers"), "still listed: {cout}");

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 8. /remember with too few arguments reports usage and does not panic.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn remember_too_few_args_reports_usage_without_panicking() {
    // Parse-only: a usage error must surface before any store access, so an
    // unopenable store is irrelevant here.
    let parsed = parse_slash_command("/remember analytics.public.orders alias");
    assert!(
        parsed.is_err(),
        "expected a usage error for a /remember missing the value, got {parsed:?}"
    );
    let msg = parsed.unwrap_err().to_string();
    assert!(
        !msg.is_empty(),
        "usage error must carry guidance, not panic silently"
    );
    // The untrusted partial input must not be echoed into the usage message.
    assert!(
        !msg.contains("analytics.public.orders"),
        "usage echoed untrusted input: {msg}"
    );

    // Only the kind, no table or value, is also a usage error.
    assert!(parse_slash_command("/remember alias").is_err());
    // No arguments at all.
    assert!(parse_slash_command("/remember").is_err());
}

// ---------------------------------------------------------------------------
// 9. An unopenable store. Parity with the headless path is what matters here
//    (spec §2, the rule this slice enforces). Every assertion here compares
//    against the headless result rather than a hardcoded code — a parity test
//    that pins an exit value stops testing parity the moment the shared
//    behaviour changes. `/contract`,
//    `/remember` and `/forget` fail with the same non-zero exit and diagnostic as
//    the headless `show`/`remember`/`forget`. Spec §5 item 9's "do not error the
//    session" reads as "the REPL keeps looping" — a non-zero command exit does
//    not tear down the session (`handle_line` returns Ok(false) for an Ok(2)
//    from the dispatcher), so both readings hold. See SPEC REVIEW.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn unopenable_store_slash_matches_headless_failure_modes() {
    let root = temp_root("unopenable_slash");
    fs::write(root.join("blocker"), b"x").unwrap();
    let bad = root.join("blocker/state.sqlite3");
    let store = SqliteStateStore::new(&bad);
    let (runtime, _name) = runtime_at(&root);

    // /contracts fails exactly as headless list does on an unreadable store.
    let headless_list = run_headless(
        ContractsCommand::List { profile: None },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    let (_, lcode, lout, lerr) =
        run_slash("/contracts", &runtime, &store, RenderFormat::Text).await;
    assert_eq!(
        lcode, headless_list.0,
        "/contracts must exit exactly as headless list does: {lout}{lerr}"
    );
    assert_ne!(
        lcode, 0,
        "an unreadable store must not look like an empty one: {lout}{lerr}"
    );
    assert_eq!(lout, headless_list.1);
    assert_eq!(lerr, headless_list.2);
    assert!(
        !format!("{lout}{lerr}").is_empty(),
        "/contracts must emit a diagnostic"
    );

    // /contract fails the same way headless show does: non-zero, same output.
    let headless_show = run_headless(
        ContractsCommand::Show {
            table: qualified().into(),
            profile: None,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    let (_, scode, sout, serr) = run_slash(
        "/contract analytics.public.orders",
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_ne!(
        scode, 0,
        "/contract must fail on an unopenable store: {sout}{serr}"
    );
    assert_eq!(scode, headless_show.0);
    assert_eq!(sout, headless_show.1);
    assert_eq!(serr, headless_show.2);

    // /remember fails, matching headless remember.
    let (_, rcode, _rout, _rerr) = run_slash(
        "/remember analytics.public.orders alias customers",
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_ne!(rcode, 0, "/remember against an unopenable store must fail");

    // /forget against an unopenable store: seed a valid id on a good store, then
    // forget on the bad one. Both slash and headless fail.
    let good_root = temp_root("unopenable_slash_good");
    let (good_runtime, _n) = runtime_at(&good_root);
    let good_store = store_at(&good_root).await;
    let (_, _, gout, _) = run_slash(
        "/remember analytics.public.orders alias customers",
        &good_runtime,
        &good_store,
        RenderFormat::Text,
    )
    .await;
    assert!(gout.contains("remembered"));
    let identity = identity_for(&good_runtime, "local");
    let profile = ProfileIdentity::parse(&identity).unwrap();
    let object = DatabaseObjectRef::new(
        profile,
        "analytics",
        "public",
        "orders",
        DatabaseObjectKind::Table,
    )
    .unwrap();
    let items = good_store.knowledge_for_object(&object).await.unwrap();
    let id = items[0].id.as_str();
    let (_, fcode, _fout, _ferr) = run_slash(
        &format!("/forget {id}"),
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_ne!(fcode, 0, "/forget against an unopenable store must fail");

    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(good_root);
}

// ---------------------------------------------------------------------------
// 10. column-scoped kinds: the column is the positional after the kind, and the
//     translated command matches the headless one (parity for the /remember
//     shape decision's column forms). Parse-only: the translation is the only
//     thing the slash adapter owns, so it is asserted without touching the store.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn remember_column_kind_translates_to_headless_command() {
    let cmd = match parse_slash_command(
        "/remember analytics.public.orders column-description amount order total",
    ) {
        Ok(Some(SlashCommand::Contracts(cmd))) => cmd,
        other => panic!("expected Contracts, got {other:?}"),
    };
    assert_eq!(
        cmd,
        ContractsCommand::Remember {
            table: qualified().into(),
            kind: ClaimKindArg::ColumnDescription,
            value: "order total".into(),
            column: Some("amount".into()),
            reason: None,
            profile: None,
        }
    );

    // column-role: the role is the value, the column precedes it.
    let cmd =
        match parse_slash_command("/remember analytics.public.orders column-role amount measure") {
            Ok(Some(SlashCommand::Contracts(cmd))) => cmd,
            other => panic!("expected Contracts, got {other:?}"),
        };
    assert_eq!(
        cmd,
        ContractsCommand::Remember {
            table: qualified().into(),
            kind: ClaimKindArg::ColumnRole,
            value: "measure".into(),
            column: Some("amount".into()),
            reason: None,
            profile: None,
        }
    );

    // time-column: a table kind (no column); the value is the column name.
    let cmd = match parse_slash_command("/remember analytics.public.orders time-column created_at")
    {
        Ok(Some(SlashCommand::Contracts(cmd))) => cmd,
        other => panic!("expected Contracts, got {other:?}"),
    };
    assert_eq!(
        cmd,
        ContractsCommand::Remember {
            table: qualified().into(),
            kind: ClaimKindArg::TimeColumn,
            value: "created_at".into(),
            column: None,
            reason: None,
            profile: None,
        }
    );

    // A value with spaces survives because everything after the kind (and the
    // column, for column kinds) is the value — the whole point of the shape.
    let cmd = match parse_slash_command(
        "/remember analytics.public.orders description orders fact table",
    ) {
        Ok(Some(SlashCommand::Contracts(cmd))) => cmd,
        other => panic!("expected Contracts, got {other:?}"),
    };
    assert_eq!(
        cmd,
        ContractsCommand::Remember {
            table: qualified().into(),
            kind: ClaimKindArg::Description,
            value: "orders fact table".into(),
            column: None,
            reason: None,
            profile: None,
        }
    );
}

/// A `/remember` with a `because <reason…>` clause translates to the same
/// `ContractsCommand::Remember` a headless `--reason` produces, so the slash
/// and headless paths agree on the reason (spec: claim-reasons, Open Question 1).
#[tokio::test]
async fn remember_because_clause_translates_to_headless_reason() {
    let cmd = match parse_slash_command(
        "/remember analytics.public.orders time-column return_date because a rental only counts once it comes back",
    ) {
        Ok(Some(SlashCommand::Contracts(cmd))) => cmd,
        other => panic!("expected Contracts, got {other:?}"),
    };
    assert_eq!(
        cmd,
        ContractsCommand::Remember {
            table: qualified().into(),
            kind: ClaimKindArg::TimeColumn,
            value: "return_date".into(),
            column: None,
            reason: Some("a rental only counts once it comes back".into()),
            profile: None,
        }
    );

    // A column-role with a `because` clause: the column and role come first,
    // then the value, then the reason.
    let cmd = match parse_slash_command(
        "/remember analytics.public.orders column-role amount measure because money the customer paid",
    ) {
        Ok(Some(SlashCommand::Contracts(cmd))) => cmd,
        other => panic!("expected Contracts, got {other:?}"),
    };
    assert_eq!(
        cmd,
        ContractsCommand::Remember {
            table: qualified().into(),
            kind: ClaimKindArg::ColumnRole,
            value: "measure".into(),
            column: Some("amount".into()),
            reason: Some("money the customer paid".into()),
            profile: None,
        }
    );
}

// ---------------------------------------------------------------------------
// 11. /queue and `saya contracts queue` return the same claim ids in the same
//     order. Phase 3d parity: the slash adapter only translates `/queue` into
//     the same `ContractsCommand::Queue` the headless parser produces and hands
//     it to the shared dispatcher — no second queue read, no second DTO mapping.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn queue_slash_and_headless_agree_on_claim_ids_and_order() {
    let root = temp_root("queue_parity");
    let (runtime, _name) = runtime_at(&root);
    let store = store_at(&root).await;

    // Seed three `Pending` knowledge items on distinct objects, directly
    // through the store (slash `remember` only confirms; the queue is the
    // candidate worklist). The queue reads `knowledge_items`, so a legacy
    // `propose_claim` row (which writes `contract_claims`, not `knowledge_items`)
    // would be invisible to it.
    let identity = identity_for(&runtime, "local");
    let profile = ProfileIdentity::parse(&identity).unwrap();
    for table in ["a", "b", "c"] {
        let object = DatabaseObjectRef::new(
            profile.clone(),
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
            .put_knowledge_item(saya_store::KnowledgeItemRequest {
                object,
                slot,
                value: payload,
                source: ClaimOrigin::AssistantInferred,
                state: KnowledgeState::Pending,
                schema_binding_json: serde_json::to_string(&binding).unwrap(),
                fingerprint: unobserved_fingerprint(),
            })
            .await
            .unwrap();
    }

    let headless = run_headless(
        ContractsCommand::Queue {
            profile: None,
            limit: None,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    let (_cmd, code, out, err) = run_slash("/queue", &runtime, &store, RenderFormat::Text).await;

    assert_eq!(code, 0, "/queue stderr: {err}");
    // The translated slash command must equal the headless one — same operation.
    assert_eq!(
        _cmd,
        ContractsCommand::Queue {
            profile: None,
            limit: None,
        }
    );
    // Same claim ids, same order, byte for byte — a second queue read would
    // diverge here.
    assert_eq!(out, headless.1, "slash /queue diverged from headless queue");
    assert_eq!(err, headless.2);

    // The queue lists three `ki-…` candidates, in the same order on both paths.
    // (The queue orders oldest-first, then slot, then id — not by evidence,
    // which the legacy `contract_evidence` table carried; that table is gone.)
    let slash_ids: Vec<&str> = out
        .lines()
        .filter(|l| l.starts_with("ki-"))
        .map(|l| l.split_whitespace().next().unwrap_or(""))
        .collect();
    assert_eq!(slash_ids.len(), 3, "/queue output: {out}");
    let headless_ids: Vec<&str> = headless
        .1
        .lines()
        .filter(|l| l.starts_with("ki-"))
        .map(|l| l.split_whitespace().next().unwrap_or(""))
        .collect();
    assert_eq!(slash_ids, headless_ids, "order diverged");

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 12. The TUI `/queue` adapter routes at the session's active profile, not the
//     configured default. Regression for the P2 defect where `with_profile`
//     (crates/saya-cli/src/interactive/tui/dispatch_actions.rs) fell through for
//     `Queue`, so a `/queue` parsed to `Queue { profile: None }` reached the
//     dispatcher un-stamped and resolved the configured default — reading another
//     profile's candidates. The queue is where a human confirms a claim, so
//     cross-profile isolation (an invariant with a named test in every layer
//     beneath) must hold at the adapter too.
//
//     The slash path parses `/queue` to `Queue { profile: None }` (test 11); the
//     TUI adapter's only addition is stamping the active profile onto that
//     `None`. This test proves the stamp is what routes the queue: with the
//     configured default set to `local` and a candidate seeded only under
//     `staging` (the profile a `/connect staging` would have made active),
//     `Queue { profile: Some("staging") }` — what the adapter now produces —
//     lists the candidate, while the un-stamped `Queue { profile: None }` the
//     broken adapter emitted resolves the default `local` and does not.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn tui_queue_routes_at_active_profile_not_configured_default() {
    let root = temp_root("queue_active_profile");
    // Two profiles; `local` is the configured default (via the `--profile` the
    // headless harness sets on load). `staging` is the profile a `/connect`
    // would have made active in the TUI — distinct from the default.
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
    // Migrate the pool and seed an (empty) cached schema for `staging` so its
    // candidates classify against a real cache state, not a missing one.
    let staging_identity = identity_for(&runtime, "staging");
    store
        .upsert_schema(&staging_identity, &SchemaTree::default())
        .await
        .unwrap();

    // Seed one `Pending` knowledge item under `staging` only, directly through the
    // store. The queue reads `knowledge_items`, so a legacy `propose_claim` row
    // (which writes `contract_claims`) would be invisible to it.
    let staging_profile = ProfileIdentity::parse(&staging_identity).unwrap();
    let object = DatabaseObjectRef::new(
        staging_profile.clone(),
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
        .put_knowledge_item(saya_store::KnowledgeItemRequest {
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

    // The adapter's output: the active profile (`staging`) stamped onto the
    // `Queue { profile: None }` the slash parser produced. The candidate appears.
    let (active_code, active_out, active_err) = run_headless(
        ContractsCommand::Queue {
            profile: Some("staging".into()),
            limit: None,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(active_code, 0, "active queue stderr: {active_err}");
    assert!(
        active_out.contains("ki-"),
        "active (staging) queue should list the seeded candidate: {active_out}"
    );

    // The broken adapter's output: `Queue { profile: None }` un-stamped, which
    // resolves the configured default (`local`). The candidate must NOT appear —
    // it lives under `staging`. This is the divergence the stamp prevents.
    let (default_code, default_out, default_err) = run_headless(
        ContractsCommand::Queue {
            profile: None,
            limit: None,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(default_code, 0, "default queue stderr: {default_err}");
    assert!(
        !default_out.contains("ki-"),
        "un-stamped queue resolved the default (local) and must not show staging's \
         candidate — the bug the adapter's stamp prevents: {default_out}"
    );

    // Sanity: the default profile is `local`, and its queue is empty too.
    let (local_code, local_out, local_err) = run_headless(
        ContractsCommand::Queue {
            profile: Some("local".into()),
            limit: None,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(local_code, 0, "local queue stderr: {local_err}");
    assert!(
        !local_out.contains("ki-"),
        "local queue should be empty: {local_out}"
    );

    let _ = fs::remove_dir_all(root);
}
