//! Phase 6b: `saya contracts import` and `saya contracts export`. Drives the
//! command adapter directly (constructs `ContractsCommand` values and calls
//! `run_contracts`), never shelling out. Covers `.claude/specs/spec-6b-import-export.md`.
//!
//! Output is captured through the thread-local seam in `output::emit` so the
//! tests assert on rendered text without racing the global stdout under parallel
//! runs.

use saya_cli::{
    ContractsCommand, RenderFormat, RuntimeConfig, capture_output_start, capture_output_take,
    load_with_sources, profile_identity, run_contracts,
};
use saya_store::{
    ClaimEvidence, ContractStore, EvidenceKind, ForgetReason, ProposeClaim, ProposeOutcome,
    SchemaStore, SqliteStateStore, object_id,
};
use saya_types::{
    ClaimOrigin, ClaimStatus, Column, ColumnRole, Database, DatabaseObjectKind, DatabaseObjectRef,
    ProfileIdentity, Schema, SchemaTree, Table,
};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

// ---------------------------------------------------------------------------
// harness — mirrors tests/contracts_cli.rs
// ---------------------------------------------------------------------------

fn temp_root(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("saya-6b-{label}-{}-{stamp}", std::process::id()));
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
    let (runtime, _) = runtime_at(root);
    let identity = identity_for(&runtime, "local");
    store
        .upsert_schema(&identity, &SchemaTree::default())
        .await
        .unwrap();
    store
}

fn identity_for(runtime: &RuntimeConfig, name: &str) -> String {
    let profile = runtime.named_profile(name).unwrap();
    profile_identity(name, profile, &runtime.cache_scope)
        .as_str()
        .to_owned()
}

fn profile_for(runtime: &RuntimeConfig, name: &str) -> ProfileIdentity {
    ProfileIdentity::parse(&identity_for(runtime, name)).unwrap()
}

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

fn contracts_dir(project: &Path) -> PathBuf {
    project.join(".saya").join("contracts")
}

fn write_contract(project: &Path, name: &str, body: &str) {
    let dir = contracts_dir(project);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(name), body).unwrap();
}

fn object_ref(runtime: &RuntimeConfig, table: &str) -> DatabaseObjectRef {
    DatabaseObjectRef::new(
        profile_for(runtime, "local"),
        "analytics",
        "public",
        table,
        DatabaseObjectKind::Table,
    )
    .unwrap()
}

/// The filename export writes for `object`: the encoded object identity
/// (`o-<64 hex>.toml`), derived — not the raw qualified name. A database
/// identifier is attacker-influenceable and may contain `/` or `..`, so the
/// filename is built from the filesystem-safe encoded identity and the
/// qualified name lives inside the file as data. See the export-escape P1.
fn export_filename(object: &DatabaseObjectRef) -> String {
    format!("{}.toml", object_id(object))
}

/// Seed a confirmed claim straight through the store, with optional evidence.
async fn seed_confirmed(
    store: &SqliteStateStore,
    runtime: &RuntimeConfig,
    table: &str,
    payload: saya_types::ClaimPayload,
    evidence: Option<ClaimEvidence>,
) {
    let request = ProposeClaim {
        object: object_ref(runtime, table),
        fingerprint: unobserved_fingerprint(),
        payload,
        origin: ClaimOrigin::UserExplicit,
        initial_status: ClaimStatus::Confirmed,
        evidence,
        referenced_columns: Vec::new(),
    };
    assert!(matches!(
        store.propose_claim(request).await.unwrap(),
        ProposeOutcome::Stored(_)
    ));
}

/// Forget the first claim for `table` whose payload equals `payload`, leaving
/// the tombstone (payload cleared, dedup key preserved). Re-proposing the same
/// fact must read as a forgotten duplicate, never "added".
async fn forget_first(
    store: &SqliteStateStore,
    runtime: &RuntimeConfig,
    table: &str,
    payload: saya_types::ClaimPayload,
) {
    let claims = store
        .list_claims(&object_ref(runtime, table), &[])
        .await
        .unwrap();
    let target = claims
        .iter()
        .find(|c| c.payload.as_ref() == Some(&payload))
        .unwrap_or_else(|| panic!("no claim with payload {payload:?} to forget"));
    store
        .forget_claim(&target.id, ForgetReason::UserRequest)
        .await
        .unwrap();
}

fn unobserved_fingerprint() -> saya_types::SchemaFingerprint {
    saya_types::SchemaFingerprint::from_parts(saya_types::FINGERPRINT_VERSION, &"0".repeat(64))
        .unwrap()
}

/// A live schema for `analytics.public.orders` with `id` and `customer_id`
/// columns, cached for the profile so import's stale check has something to
/// compare against.
async fn cache_orders_schema(store: &SqliteStateStore, runtime: &RuntimeConfig, table: &str) {
    let schema = SchemaTree {
        databases: vec![Database {
            name: "analytics".into(),
            schemas: vec![Schema {
                name: "public".into(),
                tables: vec![Table {
                    name: table.into(),
                    columns: vec![
                        Column {
                            name: "id".into(),
                            data_type: "bigint".into(),
                            nullable: false,
                        },
                        Column {
                            name: "customer_id".into(),
                            data_type: "bigint".into(),
                            nullable: false,
                        },
                    ],
                }],
            }],
        }],
    };
    store
        .upsert_schema(&identity_for(runtime, "local"), &schema)
        .await
        .unwrap();
}

// ---------------------------------------------------------------------------
// 1. --dry-run writes nothing: the store is byte-identical afterwards
// ---------------------------------------------------------------------------
#[tokio::test]
async fn dry_run_writes_nothing() {
    let root = temp_root("dryrun-noop");
    let (runtime, _) = runtime_at(&root);
    let store = store_at(&root).await;
    write_contract(
        &root,
        "orders.toml",
        "version = 1\nobject = \"analytics.public.orders\"\n[[claims]]\nkind = \"description\"\nvalue = \"orders fact table\"\n",
    );

    let before = fs::read(root.join("state.sqlite3")).unwrap();
    let import = ContractsCommand::Import {
        path: root.clone(),
        dry_run: true,
        profile: None,
    };
    let (code, out, err) = run(import, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "stderr: {err}");
    assert!(out.contains("dry run"), "out: {out}");
    assert!(out.contains("added"), "dry run reports added: {out}");

    // Read the main database file with the pool still open — the same
    // conditions as the `before` snapshot — so a WAL checkpoint between the two
    // reads cannot masquerade as a dry-run write. The dry run does reads only;
    // the on-disk main file is unchanged.
    let after = fs::read(root.join("state.sqlite3")).unwrap();
    assert_eq!(before, after, "dry run changed the state database bytes");
    // And behaviorally: nothing was stored.
    let claims = store
        .list_claims(&object_ref(&runtime, "orders"), &[])
        .await
        .unwrap();
    assert!(claims.is_empty(), "dry run stored a claim: {claims:?}");

    let _ = fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------
// 2. A dry run reports added, duplicate, conflicting and stale distinctly
// ---------------------------------------------------------------------------
#[tokio::test]
async fn dry_run_reports_four_verdicts_distinctly() {
    let root = temp_root("dryrun-classes");
    let (runtime, _) = runtime_at(&root);
    let store = store_at(&root).await;
    cache_orders_schema(&store, &runtime, "orders").await;

    // Existing confirmed claims: a duplicate-to-be and a conflict-to-be.
    seed_confirmed(
        &store,
        &runtime,
        "orders",
        saya_types::ClaimPayload::table_alias("customers").unwrap(),
        None,
    )
    .await;
    seed_confirmed(
        &store,
        &runtime,
        "orders",
        saya_types::ClaimPayload::column_role("customer_id", ColumnRole::Identifier).unwrap(),
        None,
    )
    .await;

    // One file with: a fresh description (added), the same alias (duplicate),
    // a different role for customer_id (conflicting), and a column description
    // for a column the live schema lacks (stale).
    write_contract(
        &root,
        "orders.toml",
        "\
version = 1
object = \"analytics.public.orders\"
[[claims]]
kind = \"description\"
value = \"orders fact table\"
[[claims]]
kind = \"alias\"
value = \"customers\"
[[claims]]
kind = \"column-role\"
column = \"customer_id\"
value = \"measure\"
[[claims]]
kind = \"column-description\"
column = \"ghost_col\"
value = \"no such column\"
",
    );

    let import = ContractsCommand::Import {
        path: root.clone(),
        dry_run: true,
        profile: None,
    };
    let (code, out, err) = run(import, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "stderr: {err}");
    for word in ["added", "duplicate", "conflicting", "stale"] {
        assert!(
            out.contains(&format!("  {word}  ")),
            "dry run must report {word} distinctly: {out}"
        );
    }
    assert!(
        out.contains("1 added") || out.contains("added 1"),
        "header names the added count: {out}"
    );

    // Nothing was stored by the dry run.
    let claims = store
        .list_claims(&object_ref(&runtime, "orders"), &[])
        .await
        .unwrap();
    assert_eq!(claims.len(), 2, "dry run stored claims: {claims:?}");

    let _ = fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------
// 3. A real import stores the claims; re-importing reports duplicates and
//    stores nothing new
// ---------------------------------------------------------------------------
#[tokio::test]
async fn real_import_stores_then_re_import_duplicates() {
    let root = temp_root("import-then-redup");
    let (runtime, _) = runtime_at(&root);
    let store = store_at(&root).await;
    write_contract(
        &root,
        "orders.toml",
        "version = 1\nobject = \"analytics.public.orders\"\n[[claims]]\nkind = \"description\"\nvalue = \"orders fact table\"\n[[claims]]\nkind = \"alias\"\nvalue = \"customers\"\n",
    );

    let import = || ContractsCommand::Import {
        path: root.clone(),
        dry_run: false,
        profile: None,
    };
    let (code, out, err) = run(import(), &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "stderr: {err}");
    assert!(out.contains("imported"), "out: {out}");

    let claims = store
        .list_claims(&object_ref(&runtime, "orders"), &[])
        .await
        .unwrap();
    assert_eq!(
        claims.len(),
        2,
        "import did not store both claims: {claims:?}"
    );
    assert!(
        claims.iter().all(|c| c.status == ClaimStatus::Confirmed),
        "imported claims must be confirmed: {claims:?}"
    );
    assert!(
        claims.iter().all(|c| c.origin == ClaimOrigin::TeamFile),
        "imported claims must be TeamFile: {claims:?}"
    );

    // Re-importing the same files: duplicates only, nothing new.
    let (code, out, err) = run(import(), &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "stderr: {err}");
    assert!(
        out.contains("duplicate"),
        "re-import must report duplicate: {out}"
    );
    let claims = store
        .list_claims(&object_ref(&runtime, "orders"), &[])
        .await
        .unwrap();
    assert_eq!(
        claims.len(),
        2,
        "re-import stored something new: {claims:?}"
    );

    let _ = fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------
// 3b. A forgotten claim is a tombstone: its payload is cleared but its dedup
//     key survives, so re-proposing the same fact hits the store's Duplicate
//     path and stores nothing. Before the fix the pre-scan skipped the
//     payload-free tombstone and classified the file as "added", so the report
//     told the user something was imported that was not — the tombstone
//     decision must never read as success. (Dry run and real import share the
//     same pre-scan, so both must read the forgotten duplicate.)
// ---------------------------------------------------------------------------
#[tokio::test]
async fn re_importing_a_forgotten_claim_reads_duplicate_forgotten_not_added() {
    let root = temp_root("import-forgotten-dup");
    let (runtime, _) = runtime_at(&root);
    let store = store_at(&root).await;
    let alias = saya_types::ClaimPayload::table_alias("customers").unwrap();
    seed_confirmed(&store, &runtime, "orders", alias.clone(), None).await;
    // Tombstone the alias: payload cleared, dedup key preserved.
    forget_first(&store, &runtime, "orders", alias.clone()).await;
    let before = store
        .list_claims(&object_ref(&runtime, "orders"), &[])
        .await
        .unwrap();
    assert_eq!(before.len(), 1, "{before:?}");
    assert_eq!(before[0].status, ClaimStatus::Forgotten);
    assert!(
        before[0].payload.is_none(),
        "forgotten claim is a tombstone"
    );

    write_contract(
        &root,
        "orders.toml",
        "version = 1\nobject = \"analytics.public.orders\"\n[[claims]]\nkind = \"alias\"\nvalue = \"customers\"\n",
    );
    let import = || ContractsCommand::Import {
        path: root.clone(),
        dry_run: false,
        profile: None,
    };

    // A dry run must report the forgotten duplicate, not "added".
    let dry = ContractsCommand::Import {
        path: root.clone(),
        dry_run: true,
        profile: None,
    };
    let (code, out, err) = run(dry, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "stderr: {err}");
    assert!(
        out.contains("duplicate") && out.contains("(forgotten)"),
        "dry run must report a forgotten duplicate: {out}"
    );
    assert!(
        !out.contains("0 added, 0 duplicate") && !out.contains("1 added"),
        "dry run must not read the forgotten claim as added: {out}"
    );

    // A real import must report the same and store nothing new.
    let (code, out, err) = run(import(), &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "stderr: {err}");
    assert!(
        out.contains("duplicate") && out.contains("(forgotten)"),
        "import must report a forgotten duplicate: {out}"
    );
    assert!(
        !out.contains("1 added"),
        "import must not read the forgotten claim as added: {out}"
    );
    let after = store
        .list_claims(&object_ref(&runtime, "orders"), &[])
        .await
        .unwrap();
    assert_eq!(
        after.len(),
        1,
        "re-proposing a forgotten claim must not store a new claim: {after:?}"
    );
    assert_eq!(after[0].status, ClaimStatus::Forgotten);
    assert!(
        after[0].payload.is_none(),
        "the tombstone must not be resurrected: {after:?}"
    );

    let _ = fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------
// 4. A file referencing a column the live schema lacks is stale and not stored
// ---------------------------------------------------------------------------
#[tokio::test]
async fn stale_column_is_reported_and_not_stored() {
    let root = temp_root("stale-col");
    let (runtime, _) = runtime_at(&root);
    let store = store_at(&root).await;
    cache_orders_schema(&store, &runtime, "orders").await;
    write_contract(
        &root,
        "orders.toml",
        "version = 1\nobject = \"analytics.public.orders\"\n[[claims]]\nkind = \"column-description\"\ncolumn = \"ghost_col\"\nvalue = \"no such column\"\n",
    );

    let import = ContractsCommand::Import {
        path: root.clone(),
        dry_run: false,
        profile: None,
    };
    let (code, out, err) = run(import, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "stderr: {err}");
    assert!(out.contains("stale"), "must report stale: {out}");

    let claims = store
        .list_claims(&object_ref(&runtime, "orders"), &[])
        .await
        .unwrap();
    assert!(
        claims.is_empty(),
        "a stale claim must not be stored: {claims:?}"
    );

    let _ = fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------
// 5. Export then import round-trips: the claims that come back are equivalent
// ---------------------------------------------------------------------------
#[tokio::test]
async fn export_then_import_round_trips() {
    let src = temp_root("rt-src");
    let (runtime_src, _) = runtime_at(&src);
    let store_src = store_at(&src).await;
    seed_confirmed(
        &store_src,
        &runtime_src,
        "orders",
        saya_types::ClaimPayload::table_alias("customers").unwrap(),
        None,
    )
    .await;
    seed_confirmed(
        &store_src,
        &runtime_src,
        "orders",
        saya_types::ClaimPayload::default_time_column("created_at").unwrap(),
        None,
    )
    .await;

    // Export straight into a fresh project's .saya/contracts, then import there.
    let dst = temp_root("rt-dst");
    let (runtime_dst, _) = runtime_at(&dst);
    let store_dst = store_at(&dst).await;
    let dest = contracts_dir(&dst);
    let export = ContractsCommand::Export {
        destination: dest.clone(),
        profile: None,
        force: false,
    };
    let (code, out, err) = run(export, &runtime_src, &store_src, RenderFormat::Text).await;
    assert_eq!(code, 0, "export stderr: {err}");
    assert!(out.contains("exported"), "export out: {out}");
    let orders = object_ref(&runtime_src, "orders");
    assert!(dest.join(export_filename(&orders)).exists());

    let import = ContractsCommand::Import {
        path: dst.clone(),
        dry_run: false,
        profile: None,
    };
    let (code, _out, err) = run(import, &runtime_dst, &store_dst, RenderFormat::Text).await;
    assert_eq!(code, 0, "import stderr: {err}");

    let imported = store_dst
        .list_claims(&object_ref(&runtime_dst, "orders"), &[])
        .await
        .unwrap();
    assert_eq!(imported.len(), 2, "round-trip lost claims: {imported:?}");
    assert!(
        imported.iter().all(|c| c.status == ClaimStatus::Confirmed),
        "round-tripped claims must be confirmed: {imported:?}"
    );
    // Equivalence: the same alias and the same time-column value came back.
    let payloads: Vec<_> = imported.iter().filter_map(|c| c.payload.clone()).collect();
    assert!(
        payloads
            .iter()
            .any(|p| matches!(p, saya_types::ClaimPayload::TableAlias { alias, .. } if alias == "customers")),
        "alias did not round-trip: {payloads:?}"
    );
    assert!(
        payloads
            .iter()
            .any(|p| matches!(p, saya_types::ClaimPayload::DefaultTimeColumn { column, .. } if column == "created_at")),
        "time-column did not round-trip: {payloads:?}"
    );

    let _ = fs::remove_dir_all(&src);
    let _ = fs::remove_dir_all(&dst);
}

// ---------------------------------------------------------------------------
// 6. An exported file contains no opaque identity, no evidence, no absolute path
// ---------------------------------------------------------------------------
#[tokio::test]
async fn export_leaks_no_identity_evidence_or_absolute_path() {
    let root = temp_root("export-leak");
    let (runtime, _) = runtime_at(&root);
    let store = store_at(&root).await;

    // Plant a confirmed claim carrying evidence (a session id and turn ordinal).
    let evidence = ClaimEvidence {
        kind: EvidenceKind::ExplicitUserStatement,
        session_id: Some("sess-SENTINEL-6b".into()),
        turn_ordinal: Some(7),
        observed_unix_ms: 1_700_000_000_000,
    };
    seed_confirmed(
        &store,
        &runtime,
        "orders",
        saya_types::ClaimPayload::table_alias("customers").unwrap(),
        Some(evidence),
    )
    .await;

    let dest = root.join("out");
    // The claim really does carry the evidence in the store — otherwise the
    // byte scan below would prove nothing. The export must omit it, not the
    // seed.
    let stored = store
        .list_claims(&object_ref(&runtime, "orders"), &[])
        .await
        .unwrap();
    assert_eq!(stored.len(), 1, "seeded claim missing: {stored:?}");
    let evidence_count = store.evidence_count(&stored[0].id).await.unwrap();
    assert!(
        evidence_count >= 1,
        "evidence was not stored, so the leak scan is meaningless"
    );

    let export = ContractsCommand::Export {
        destination: dest.clone(),
        profile: None,
        force: false,
    };
    let (code, out, err) = run(export, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "export stderr: {err}");
    let file = dest.join(export_filename(&object_ref(&runtime, "orders")));
    let bytes = fs::read_to_string(&file).unwrap();

    let identity = identity_for(&runtime, "local");
    assert!(
        !bytes.contains(&identity),
        "opaque identity leaked into export: {bytes}"
    );
    assert!(
        !bytes.contains("sess-SENTINEL-6b"),
        "session id leaked into export: {bytes}"
    );
    assert!(
        !bytes.contains("turn"),
        "evidence turn ordinal leaked into export: {bytes}"
    );
    let root_str = root.display().to_string();
    assert!(
        !bytes.contains(&root_str),
        "absolute temp root leaked into export: {bytes}"
    );
    // The object is written as its qualified name, no profile-identity prefix.
    assert!(
        bytes.contains("object = \"analytics.public.orders\""),
        "object must be the qualified name: {bytes}"
    );
    // The report itself must not leak the identity either.
    assert!(
        !out.contains(&identity),
        "opaque identity leaked into export report: {out}"
    );

    let _ = fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------
// 7. Export writes confirmed claims only — a candidate does not appear
// ---------------------------------------------------------------------------
#[tokio::test]
async fn export_writes_confirmed_only_candidate_absent() {
    let root = temp_root("export-confirmed-only");
    let (runtime, _) = runtime_at(&root);
    let store = store_at(&root).await;

    // A candidate for `candidates` — an object with no confirmed claim.
    let candidate_request = ProposeClaim {
        object: object_ref(&runtime, "candidates"),
        fingerprint: unobserved_fingerprint(),
        payload: saya_types::ClaimPayload::table_alias("cand").unwrap(),
        origin: ClaimOrigin::AssistantInferred,
        initial_status: ClaimStatus::Candidate,
        evidence: None,
        referenced_columns: Vec::new(),
    };
    assert!(matches!(
        store.propose_claim(candidate_request).await.unwrap(),
        ProposeOutcome::Stored(_)
    ));
    // A confirmed claim for `orders` — this one must be exported.
    seed_confirmed(
        &store,
        &runtime,
        "orders",
        saya_types::ClaimPayload::table_alias("customers").unwrap(),
        None,
    )
    .await;

    let dest = root.join("out");
    let export = ContractsCommand::Export {
        destination: dest.clone(),
        profile: None,
        force: false,
    };
    let (code, out, err) = run(export, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "export stderr: {err}");
    let orders = object_ref(&runtime, "orders");
    let candidates = object_ref(&runtime, "candidates");
    assert!(dest.join(export_filename(&orders)).exists());
    assert!(
        !dest.join(export_filename(&candidates)).exists(),
        "a candidate must not be exported: {out}"
    );
    let orders_bytes = fs::read_to_string(dest.join(export_filename(&orders))).unwrap();
    assert!(
        !orders_bytes.contains("cand"),
        "candidate value leaked into another object's file: {orders_bytes}"
    );

    let _ = fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------
// 8. Export is atomic: no partial file remains if writing fails midway
// ---------------------------------------------------------------------------
#[tokio::test]
async fn export_is_atomic_no_temp_left_on_failure() {
    let root = temp_root("export-atomic");
    let (runtime, _) = runtime_at(&root);
    let store = store_at(&root).await;
    seed_confirmed(
        &store,
        &runtime,
        "orders",
        saya_types::ClaimPayload::table_alias("customers").unwrap(),
        None,
    )
    .await;

    // A read-only parent directory: file creation inside it must fail.
    let readonly_parent = root.join("readonly");
    fs::create_dir_all(&readonly_parent).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&readonly_parent, fs::Permissions::from_mode(0o555)).unwrap();
    }
    let dest = readonly_parent.join("contracts");

    let export = ContractsCommand::Export {
        destination: dest.clone(),
        profile: None,
        force: false,
    };
    let (code, _out, err) = run(export, &runtime, &store, RenderFormat::Text).await;
    assert_ne!(code, 0, "export to a read-only parent must fail: {err}");

    // No temp file is left behind in the read-only parent.
    let leftovers: Vec<_> = fs::read_dir(&readonly_parent)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        !leftovers.iter().any(|n| n.contains("saya-export")),
        "a temp file was left behind: {leftovers:?}"
    );
    assert!(
        !dest.exists() || fs::read_dir(&dest).unwrap().next().is_none(),
        "a partial destination was written: {leftovers:?}"
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&readonly_parent, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let _ = fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------
// 9. Import with an unreadable store fails non-zero; a dry run against one too
// ---------------------------------------------------------------------------
#[tokio::test]
async fn unreadable_store_import_and_dry_run_fail_nonzero() {
    let root = temp_root("unopenable");
    fs::write(root.join("blocker"), b"x").unwrap();
    let bad = root.join("blocker/state.sqlite3");
    let store = SqliteStateStore::new(&bad);
    let (runtime, _) = runtime_at(&root);
    write_contract(
        &root,
        "orders.toml",
        "version = 1\nobject = \"analytics.public.orders\"\n[[claims]]\nkind = \"description\"\nvalue = \"orders fact table\"\n",
    );

    let import = ContractsCommand::Import {
        path: root.clone(),
        dry_run: false,
        profile: None,
    };
    let (code, out, err) = run(import, &runtime, &store, RenderFormat::Text).await;
    assert_ne!(
        code, 0,
        "import must fail on an unopenable store: {out}{err}"
    );
    assert!(
        !format!("{out}{err}").is_empty(),
        "import must emit a diagnostic on an unopenable store"
    );

    let dry = ContractsCommand::Import {
        path: root.clone(),
        dry_run: true,
        profile: None,
    };
    let (code, out, err) = run(dry, &runtime, &store, RenderFormat::Text).await;
    assert_ne!(
        code, 0,
        "a dry run must also fail on an unopenable store: {out}{err}"
    );

    // Export against the same unopenable store fails non-zero too.
    let export = ContractsCommand::Export {
        destination: root.join("out"),
        profile: None,
        force: false,
    };
    let (code, out, err) = run(export, &runtime, &store, RenderFormat::Text).await;
    assert_ne!(
        code, 0,
        "export must fail on an unopenable store: {out}{err}"
    );

    let _ = fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------
// 10. Symptom B on the import path: an imported claim for an object the cached
//     schema contains (and whose referenced columns it contains) must store
//     the real digest and read `current`, not `needs_review`. Before the fix
//     `propose_imported` stored the all-zeros unobserved sentinel for every
//     Added claim, so a valid imported fact read `needs_review` the moment it
//     landed — the same bug `remember` had. Import already refused absent
//     objects (symptom A) via the Stale verdict, so only symptom B needed
//     fixing here.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn import_added_claim_against_cached_schema_reads_current_not_needs_review() {
    let root = temp_root("import_current_not_needs_review");
    let (runtime, _) = runtime_at(&root);
    let store = store_at(&root).await;
    // Cache `analytics.public.orders` with a `created_at` column the claim
    // references — what `connection schema local --refresh` would write.
    let table = Table {
        name: "orders".into(),
        columns: vec![
            Column {
                name: "id".into(),
                data_type: "bigint".into(),
                nullable: false,
            },
            Column {
                name: "created_at".into(),
                data_type: "timestamp".into(),
                nullable: false,
            },
        ],
    };
    let schema = SchemaTree {
        databases: vec![Database {
            name: "analytics".into(),
            schemas: vec![Schema {
                name: "public".into(),
                tables: vec![table.clone()],
            }],
        }],
    };
    store
        .upsert_schema(&identity_for(&runtime, "local"), &schema)
        .await
        .unwrap();

    // A default-time-column claim for `created_at` — object present, column
    // present, so import classifies it Added and stores it.
    write_contract(
        &root,
        "orders.toml",
        "version = 1\nobject = \"analytics.public.orders\"\n[[claims]]\nkind = \"time-column\"\nvalue = \"created_at\"\n",
    );
    let import = ContractsCommand::Import {
        path: root.clone(),
        dry_run: false,
        profile: None,
    };
    let (code, out, err) = run(import, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "import stderr: {err}");
    assert!(out.contains("added"), "claim must be added: {out}");

    // The stored claim's fingerprint is the cached table's real digest, not
    // the all-zeros sentinel — the load-bearing assertion.
    let claims = store
        .list_claims(&object_ref(&runtime, "orders"), &[])
        .await
        .unwrap();
    assert_eq!(claims.len(), 1, "{claims:?}");
    let real = saya_types::SchemaFingerprint::of_table(DatabaseObjectKind::Table, &table);
    assert_eq!(
        claims[0].schema_fingerprint, real,
        "import stored the unobserved sentinel, not the cached table's real digest"
    );
    // And the column snapshot carries the resolved type, not the empty type.
    assert_eq!(claims[0].referenced_columns.len(), 1, "{claims:?}");
    assert_eq!(claims[0].referenced_columns[0].name, "created_at");
    assert_eq!(
        claims[0].referenced_columns[0].data_type, "timestamp",
        "imported column snapshot must carry the cached type: {:?}",
        claims[0].referenced_columns
    );

    // `show` reads `current`, not `needs_review`.
    let show = ContractsCommand::Show {
        table: "analytics.public.orders".into(),
        profile: None,
    };
    let (code, out, err) = run(show, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "show stderr: {err}");
    assert!(
        out.contains("[current]"),
        "imported claim must read current: {out}"
    );
    assert!(
        !out.contains("needs_review"),
        "a valid imported claim must not read needs_review: {out}"
    );

    let _ = fs::remove_dir_all(&root);
}

// A table-level imported claim (no referenced columns) against a cached
// schema must also read `current`: before the fix the all-zeros sentinel made
// even a table-level claim read `needs_review` (fingerprint moved, no columns
// to break → NeedsReview).
#[tokio::test]
async fn import_table_level_claim_against_cached_schema_reads_current() {
    let root = temp_root("import_table_current");
    let (runtime, _) = runtime_at(&root);
    let store = store_at(&root).await;
    let table = Table {
        name: "orders".into(),
        columns: vec![Column {
            name: "id".into(),
            data_type: "bigint".into(),
            nullable: false,
        }],
    };
    let schema = SchemaTree {
        databases: vec![Database {
            name: "analytics".into(),
            schemas: vec![Schema {
                name: "public".into(),
                tables: vec![table.clone()],
            }],
        }],
    };
    store
        .upsert_schema(&identity_for(&runtime, "local"), &schema)
        .await
        .unwrap();

    write_contract(
        &root,
        "orders.toml",
        "version = 1\nobject = \"analytics.public.orders\"\n[[claims]]\nkind = \"alias\"\nvalue = \"customers\"\n",
    );
    let import = ContractsCommand::Import {
        path: root.clone(),
        dry_run: false,
        profile: None,
    };
    let (code, out, err) = run(import, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "import stderr: {err}");
    assert!(out.contains("added"), "claim must be added: {out}");

    let claims = store
        .list_claims(&object_ref(&runtime, "orders"), &[])
        .await
        .unwrap();
    assert_eq!(claims.len(), 1, "{claims:?}");
    let real = saya_types::SchemaFingerprint::of_table(DatabaseObjectKind::Table, &table);
    assert_eq!(
        claims[0].schema_fingerprint, real,
        "table-level import must store the real digest, not the sentinel"
    );

    let show = ContractsCommand::Show {
        table: "analytics.public.orders".into(),
        profile: None,
    };
    let (code, out, err) = run(show, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "show stderr: {err}");
    assert!(
        out.contains("[current]"),
        "table-level import must read current: {out}"
    );
    assert!(
        !out.contains("needs_review"),
        "table-level import must not read needs_review: {out}"
    );

    let _ = fs::remove_dir_all(&root);
}

// Import with no cached schema keeps the original behaviour: the unobserved
// sentinel. There is nothing to compute a real digest against, and refusing
// would make import unusable before a first refresh. (Import already reports
// Stale only against a non-empty cache; with no cache every claim is Added.)
#[tokio::test]
async fn import_with_no_cached_schema_keeps_sentinel() {
    let root = temp_root("import_no_cache_sentinel");
    let (runtime, _) = runtime_at(&root);
    let store = store_at(&root).await;
    // Drop the empty default `store_at` cached so there is genuinely no cache.
    store
        .invalidate_schema(&identity_for(&runtime, "local"))
        .await
        .unwrap();

    write_contract(
        &root,
        "orders.toml",
        "version = 1\nobject = \"analytics.public.orders\"\n[[claims]]\nkind = \"alias\"\nvalue = \"customers\"\n",
    );
    let import = ContractsCommand::Import {
        path: root.clone(),
        dry_run: false,
        profile: None,
    };
    let (code, out, err) = run(import, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "import with no cache must succeed: {err}");
    assert!(out.contains("added"), "claim must be added: {out}");

    let claims = store
        .list_claims(&object_ref(&runtime, "orders"), &[])
        .await
        .unwrap();
    assert_eq!(claims.len(), 1, "{claims:?}");
    assert_eq!(
        claims[0].schema_fingerprint,
        unobserved_fingerprint(),
        "no-cache import must store the unobserved sentinel, not a real digest"
    );

    let _ = fs::remove_dir_all(&root);
}
