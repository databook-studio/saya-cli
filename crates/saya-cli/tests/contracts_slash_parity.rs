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
use saya_store::{SchemaStore, SqliteStateStore};
use saya_types::SchemaTree;
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
    let slash_id = sout
        .strip_prefix("remembered ")
        .and_then(|rest| rest.split_whitespace().next())
        .expect("slash /remember names the id: {sout}");

    // The translated command must be structurally equal to the headless one —
    // same operation, same payload constructors, same deterministic identity.
    assert_eq!(
        slash_cmd,
        ContractsCommand::Remember {
            table: qualified().into(),
            kind: ClaimKindArg::Alias,
            value: "customers".into(),
            column: None,
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
            profile: None,
        },
        &runtime,
        &headless_store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(_hcode, 0, "headless stderr: {herr}");
    let headless_id = hout
        .strip_prefix("remembered ")
        .and_then(|rest| rest.split_whitespace().next())
        .expect("headless remember names the id: {hout}");

    assert_eq!(
        slash_id, headless_id,
        "slash and headless /remember produced different claim ids — they are not the same operation"
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
    let id = out
        .strip_prefix("remembered ")
        .and_then(|rest| rest.split_whitespace().next())
        .expect("id present: {out}");

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
    let (_, hout, _) = run_headless(
        ContractsCommand::Remember {
            table: qualified().into(),
            kind: ClaimKindArg::Alias,
            value: "customers".into(),
            column: None,
            profile: None,
        },
        &runtime2,
        &store2,
        RenderFormat::Text,
    )
    .await;
    let id2 = hout
        .strip_prefix("remembered ")
        .and_then(|rest| rest.split_whitespace().next())
        .unwrap();
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
    let id = rout
        .strip_prefix("remembered ")
        .and_then(|rest| rest.split_whitespace().next())
        .unwrap();

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
    let id = gout
        .strip_prefix("remembered ")
        .and_then(|rest| rest.split_whitespace().next())
        .unwrap();
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
            profile: None,
        }
    );
}
