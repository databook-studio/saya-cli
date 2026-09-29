//! Portable reviewed business context (B2c, ADR 0006): `saya contracts
//! export|import|import-dbt` and the `/contracts export|import|import-dbt`
//! slash forms.
//!
//! The export must carry only Active claims with identity stripped; the import
//! must validate the whole document before any store access, map logical
//! objects onto the chosen profile's schema only, land every written item as
//! Pending with no imported authority, treat conflicts and unresolvable
//! objects as reports rather than writes, and be idempotent and atomic. All
//! three operations go through the same `ContractsCommand` the slash adapter
//! translates, so the parity claim is checked by parsing, not by trusting the
//! dispatcher.
//!
//! Output is captured through the thread-local seam in `output::emit`, exactly
//! as `tests/contracts_cli.rs` does.

use saya_cli::{
    ClaimKindArg, ContractsCommand, RenderFormat, RuntimeConfig, SlashCommand,
    capture_output_start, capture_output_take, load_with_sources, parse_slash_command,
    profile_identity, run_contracts,
};
use saya_config::MemoryMode;
use saya_store::{KnowledgeItemRequest, KnowledgeItemStore, SchemaStore, SqliteStateStore};
use saya_types::{
    CONTEXT_FORMAT, CONTEXT_FORMAT_VERSION, ClaimOrigin, ClaimPayload, Column, ContextDocumentV1,
    ContextItem, Database, DatabaseObjectKind, DatabaseObjectRef, FINGERPRINT_VERSION,
    KnowledgeSlot, KnowledgeState, PortableObject, PortablePayload, ProfileIdentity, Schema,
    SchemaBinding, SchemaFingerprint, SchemaTree, Table,
};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

// ---------------------------------------------------------------------------
// harness — mirrors tests/contracts_cli.rs so both suites share one store shape
// and the derived profile identity is identical across them.
// ---------------------------------------------------------------------------

fn temp_root(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "saya-contracts-portable-{label}-{}-{stamp}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

/// A runtime over a sqlite database file. With `extra` profiles the file names
/// more than one profile; each gets its own database file so two profiles never
/// alias one schema. The connections path *is* the cache scope, so the derived
/// identity is deterministic per root.
fn runtime_with(root: &Path, extra: &[&str]) -> RuntimeConfig {
    let database = root.join("data.sqlite3");
    fs::write(&database, b"").unwrap();
    let mut connections = format!(
        "[profiles.local]\ntype = 'sqlite'\npath = '{}'\n",
        database.display()
    );
    for name in extra {
        let file = root.join(format!("data-{name}.sqlite3"));
        fs::write(&file, b"").unwrap();
        connections.push_str(&format!(
            "\n[profiles.{name}]\ntype = 'sqlite'\npath = '{}'\n",
            file.display()
        ));
    }
    fs::write(root.join("connections.toml"), connections).unwrap();
    // More than one profile needs an explicit default in config.toml.
    if !extra.is_empty() {
        fs::write(root.join("config.toml"), "default_profile = \"local\"\n").unwrap();
    }
    let options = saya_cli::GlobalOptions {
        connections: Some(root.join("connections.toml")),
        ..Default::default()
    };
    load_with_sources(&options, root, root, BTreeMap::new()).unwrap()
}

fn runtime_at(root: &Path) -> RuntimeConfig {
    runtime_with(root, &[])
}

/// A store whose migrations have run, at `root/state.sqlite3`.
async fn store_at(root: &Path, runtime: &RuntimeConfig) -> SqliteStateStore {
    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    // Touch the pool so migrations run, seeding an empty default cache for the
    // `local` identity exactly as `contracts_cli.rs` does.
    let identity = identity_for(runtime, "local");
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

// ---------------------------------------------------------------------------
// schema + claim seeds
// ---------------------------------------------------------------------------

fn orders_schema(tables: &[&str]) -> SchemaTree {
    SchemaTree {
        databases: vec![Database {
            name: "analytics".into(),
            schemas: vec![Schema {
                name: "public".into(),
                tables: tables
                    .iter()
                    .map(|name| Table {
                        name: (*name).into(),
                        columns: vec![Column {
                            name: "id".into(),
                            data_type: "bigint".into(),
                            nullable: false,
                        }],
                        primary_key: vec![],
                        foreign_keys: vec![],
                    })
                    .collect(),
            }],
        }],
    }
}

/// A schema shaped like the dbt v12 fixture's models and source.
fn dbt_schema() -> SchemaTree {
    SchemaTree {
        databases: vec![
            Database {
                name: "analytics".into(),
                schemas: vec![Schema {
                    name: "core".into(),
                    tables: ["public_orders", "customers", "hidden"]
                        .iter()
                        .map(|name| Table {
                            name: (*name).into(),
                            columns: vec![Column {
                                name: "customer_id".into(),
                                data_type: "integer".into(),
                                nullable: true,
                            }],
                            primary_key: vec![],
                            foreign_keys: vec![],
                        })
                        .collect(),
                }],
            },
            Database {
                name: "raw".into(),
                schemas: vec![Schema {
                    name: "jaffle".into(),
                    tables: vec![Table {
                        name: "raw_events".into(),
                        columns: vec![Column {
                            name: "event_key".into(),
                            data_type: "text".into(),
                            nullable: true,
                        }],
                        primary_key: vec![],
                        foreign_keys: vec![],
                    }],
                }],
            },
        ],
    }
}

/// Remembers a claim through the real `remember` adapter, so the exported
/// Active claim is the row the ordinary write path produces.
async fn remember_active(
    store: &SqliteStateStore,
    runtime: &RuntimeConfig,
    kind: ClaimKindArg,
    value: &str,
    column: Option<&str>,
) {
    let command = ContractsCommand::Remember {
        table: "analytics.public.orders".into(),
        kind,
        value: value.into(),
        column: column.map(str::to_string),
        reason: None,
        profile: None,
    };
    let (code, out, err) = run(command, runtime, store, RenderFormat::Text).await;
    assert_eq!(code, 0, "remember failed: {out}{err}");
}

/// Seeds a non-Active row directly, to prove the export excludes it.
async fn seed_other_state(
    store: &SqliteStateStore,
    runtime: &RuntimeConfig,
    state: KnowledgeState,
) {
    let profile = ProfileIdentity::parse(&identity_for(runtime, "local")).unwrap();
    let object = DatabaseObjectRef::new(
        profile,
        "analytics",
        "public",
        "orders",
        DatabaseObjectKind::Table,
    )
    .unwrap();
    let payload = ClaimPayload::table_alias("ghost-alias").unwrap();
    let slot = KnowledgeSlot::TableAlias;
    let binding = SchemaBinding::derive(&slot, &payload).unwrap();
    store
        .put_knowledge_item(KnowledgeItemRequest {
            object,
            slot,
            value: payload,
            source: ClaimOrigin::AssistantInferred,
            state,
            schema_binding_json: serde_json::to_string(&binding).unwrap(),
            fingerprint: SchemaFingerprint::from_parts(FINGERPRINT_VERSION, &"0".repeat(64))
                .unwrap(),
        })
        .await
        .unwrap();
}

/// A stored item for `object`, for state assertions. `qualified` is
/// `catalog.schema.object` as the mapping resolved it.
async fn stored_items(
    store: &SqliteStateStore,
    runtime: &RuntimeConfig,
    profile: Option<&str>,
    qualified: &str,
) -> Vec<saya_store::KnowledgeItem> {
    let identity = identity_for(runtime, profile.unwrap_or("local"));
    let profile_id = ProfileIdentity::parse(&identity).unwrap();
    let mut parts = qualified.split('.');
    let (catalog, schema, object) = match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(c), Some(s), Some(o), None) => (c, s, o),
        _ => panic!("qualified must be catalog.schema.object: {qualified}"),
    };
    let object = DatabaseObjectRef::new(
        profile_id,
        catalog,
        schema,
        object,
        DatabaseObjectKind::Table,
    )
    .unwrap();
    store.knowledge_for_object(&object).await.unwrap()
}

// ---------------------------------------------------------------------------
// context-document builders
// ---------------------------------------------------------------------------

fn context_item(catalog: &str, schema: &str, name: &str, payload: &ClaimPayload) -> ContextItem {
    ContextItem {
        object: PortableObject {
            catalog: Some(catalog.into()),
            schema: Some(schema.into()),
            name: name.into(),
            kind: DatabaseObjectKind::Table,
        },
        payload: PortablePayload::from_claim(payload).unwrap(),
        origin_note: None,
    }
}

fn context_document(items: Vec<ContextItem>) -> String {
    ContextDocumentV1 {
        format: CONTEXT_FORMAT.into(),
        version: CONTEXT_FORMAT_VERSION,
        exported_unix_ms: 0,
        items,
    }
    .to_json_pretty()
    .unwrap()
}

fn write_context_file(path: &Path, json: &str) {
    fs::write(path, json).unwrap();
}

// ---------------------------------------------------------------------------
// export: Active only, kinds counted, overwrite/symlink/dir refusals
// ---------------------------------------------------------------------------

#[tokio::test]
async fn export_writes_active_claims_only_and_counts_by_kind() {
    let root = temp_root("export_active");
    let runtime = runtime_at(&root);
    let store = store_at(&root, &runtime).await;

    remember_active(&store, &runtime, ClaimKindArg::Alias, "customers", None).await;
    remember_active(
        &store,
        &runtime,
        ClaimKindArg::Description,
        "Orders from the ERP.",
        None,
    )
    .await;
    remember_active(
        &store,
        &runtime,
        ClaimKindArg::TimeColumn,
        "return_date",
        None,
    )
    .await;
    // A pending and a dismissed row must never ride along: the file carries
    // Active claims only.
    seed_other_state(&store, &runtime, KnowledgeState::Pending).await;
    seed_other_state(&store, &runtime, KnowledgeState::Dismissed).await;

    let path = root.join("context.json");
    let (code, out, err) = run(
        ContractsCommand::Export {
            path: path.clone(),
            profile: None,
            overwrite: false,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(code, 0, "export failed: {out}{err}");
    assert!(out.contains("exported 3 claims"), "summary counts: {out}");
    assert!(out.contains("table_alias 1"), "per-kind count: {out}");
    assert!(out.contains("table_description 1"), "per-kind count: {out}");
    assert!(
        out.contains("default_time_column 1"),
        "per-kind count: {out}"
    );
    assert!(
        out.contains("Review the file before sharing."),
        "the sharing caution: {out}"
    );
    assert!(
        !out.contains("ghost-alias"),
        "non-active claims excluded: {out}"
    );

    // The file is a valid document whose items are exactly the Active claims,
    // with the opaque identity stripped.
    let document = ContextDocumentV1::from_json_bytes(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(document.items.len(), 3);
    let identity = identity_for(&runtime, "local");
    assert!(
        !fs::read_to_string(&path).unwrap().contains(&identity),
        "the opaque identity must never reach the file"
    );
    assert!(
        document
            .items
            .iter()
            .all(|item| item.object.catalog.as_deref() == Some("analytics")
                && item.object.schema.as_deref() == Some("public")),
        "objects carry their logical names: {:?}",
        document.items
    );

    // A second export onto the existing file is refused without --overwrite.
    let (code, out, err) = run(
        ContractsCommand::Export {
            path: path.clone(),
            profile: None,
            overwrite: false,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_ne!(
        code, 0,
        "existing file refused without --overwrite: {out}{err}"
    );
    let (code, out, err) = run(
        ContractsCommand::Export {
            path: path.clone(),
            profile: None,
            overwrite: true,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(code, 0, "--overwrite replaces: {out}{err}");
    let replaced = ContextDocumentV1::from_json_bytes(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(replaced.items.len(), 3);

    drop(store);
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn export_refuses_symlink_and_directory_targets() {
    let root = temp_root("export_targets");
    let runtime = runtime_at(&root);
    let store = store_at(&root, &runtime).await;
    remember_active(&store, &runtime, ClaimKindArg::Alias, "customers", None).await;

    let dir = root.join("adir");
    fs::create_dir_all(&dir).unwrap();
    let (code, out, err) = run(
        ContractsCommand::Export {
            path: dir.clone(),
            profile: None,
            overwrite: false,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_ne!(code, 0, "a directory target is refused: {out}{err}");

    // A symlink is refused even when --overwrite is passed: the atomic rename
    // would replace the link, not the file it points at, so the check is not
    // optional.
    let target = root.join("real.json");
    fs::write(&target, b"{}").unwrap();
    let link = root.join("link.json");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&target, &link).unwrap();
    #[cfg(not(unix))]
    fs::copy(&target, &link).unwrap();
    let (code, out, err) = run(
        ContractsCommand::Export {
            path: link.clone(),
            profile: None,
            overwrite: true,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    if cfg!(unix) {
        assert_ne!(code, 0, "a symlink target is refused: {out}{err}");
        assert!(
            out.contains("symbolic link") || err.contains("symbolic link"),
            "the refusal names the reason: {out}{err}"
        );
        assert_eq!(
            fs::read(&target).unwrap(),
            b"{}",
            "the link target untouched"
        );
    }

    drop(store);
    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// import: pending-not-authority, mapping, conflicts, idempotency, atomicity
// ---------------------------------------------------------------------------

/// With no cached schema the mapping fetches the live schema and the report
/// names `live` — the two schema sources stay distinguishable. The empty test
/// database discovers no tables, so the items are unavailable and nothing is
/// written.
#[tokio::test]
async fn import_without_a_cached_schema_fetches_live_and_names_the_source() {
    let root = temp_root("live_fetch");
    let runtime = runtime_at(&root);
    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    // Migrations run on first use; the schema cache stays untouched (missing).

    let items = vec![context_item(
        "analytics",
        "public",
        "orders",
        &ClaimPayload::table_description("One row per confirmed order.").unwrap(),
    )];
    let path = root.join("context.json");
    write_context_file(&path, &context_document(items));

    let (code, out, err) = run(
        ContractsCommand::Import {
            path,
            profile: None,
            preview: false,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_ne!(code, 0, "nothing landed: {out}{err}");
    assert!(
        out.contains("schema: live"),
        "the live fetch is named: {out}"
    );
    assert!(out.contains("unavailable 1"), "{out}");
    assert!(
        stored_items(&store, &runtime, None, "analytics.public.orders")
            .await
            .is_empty(),
        "an import against an empty live schema writes nothing"
    );

    drop(store);
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn imported_confirmation_is_not_authority() {
    let source_root = temp_root("authority_source");
    let source_runtime = runtime_at(&source_root);
    let source_store = store_at(&source_root, &source_runtime).await;
    remember_active(
        &source_store,
        &source_runtime,
        ClaimKindArg::Description,
        "One row per confirmed order.",
        None,
    )
    .await;
    let export_path = source_root.join("context.json");
    let (code, out, err) = run(
        ContractsCommand::Export {
            path: export_path.clone(),
            profile: None,
            overwrite: false,
        },
        &source_runtime,
        &source_store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(code, 0, "export failed: {out}{err}");

    // A second, independent root imports the file. Its schema cache names the
    // same table so the objects resolve.
    let target_root = temp_root("authority_target");
    let target_runtime = runtime_at(&target_root);
    let target_store = store_at(&target_root, &target_runtime).await;
    let identity = identity_for(&target_runtime, "local");
    target_store
        .upsert_schema(&identity, &orders_schema(&["orders"]))
        .await
        .unwrap();

    let (code, out, err) = run(
        ContractsCommand::Import {
            path: export_path.clone(),
            profile: None,
            preview: false,
        },
        &target_runtime,
        &target_store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(code, 0, "import failed: {out}{err}");
    assert!(out.contains("imported 1"), "summary: {out}");
    assert!(
        out.contains("Imported items are pending review: saya contracts queue"),
        "the queue pointer: {out}"
    );
    // The opaque identity of neither root may appear.
    let source_identity = identity_for(&source_runtime, "local");
    assert!(
        !out.contains(&source_identity) && !out.contains(&identity),
        "opaque identity leaked into import output: {out}"
    );

    // The written item is Pending under the TARGET profile's identity — an
    // imported confirmation is never an Active one.
    let items = stored_items(
        &target_store,
        &target_runtime,
        None,
        "analytics.public.orders",
    )
    .await;
    assert_eq!(items.len(), 1, "{items:?}");
    assert_eq!(items[0].state, KnowledgeState::Pending, "{items:?}");
    assert_eq!(items[0].source, ClaimOrigin::TeamFile, "{items:?}");

    // Recall reads Active claims only, so `contracts list` must show nothing
    // for the imported item.
    let (code, out, err) = run(
        ContractsCommand::List { profile: None },
        &target_runtime,
        &target_store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(code, 0, "list failed: {out}{err}");
    assert!(
        !out.contains("One row per confirmed order"),
        "a pending import must not recall as confirmed: {out}"
    );

    drop(target_store);
    drop(source_store);
    let _ = fs::remove_dir_all(source_root);
    let _ = fs::remove_dir_all(target_root);
}

#[tokio::test]
async fn context_profile_mapping_cannot_cross_connections() {
    let root = temp_root("cross_connections");
    let runtime = runtime_with(&root, &["other"]);
    let store = store_at(&root, &runtime).await;
    // `other` caches a schema naming `orders`; `local` does not.
    let other_identity = identity_for(&runtime, "other");
    store
        .upsert_schema(&other_identity, &orders_schema(&["orders"]))
        .await
        .unwrap();

    let items = vec![context_item(
        "analytics",
        "public",
        "orders",
        &ClaimPayload::table_description("One row per confirmed order.").unwrap(),
    )];
    let path = root.join("context.json");
    write_context_file(&path, &context_document(items));

    // Import against `other`: the object resolves there, and the item must be
    // keyed by `other`'s identity.
    let (code, out, err) = run(
        ContractsCommand::Import {
            path: path.clone(),
            profile: Some("other".into()),
            preview: false,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(code, 0, "import onto `other` failed: {out}{err}");
    assert!(out.contains("imported 1"), "{out}");
    assert!(
        out.contains("schema: cached"),
        "the schema source is stated: {out}"
    );
    let items = stored_items(&store, &runtime, Some("other"), "analytics.public.orders").await;
    assert_eq!(
        items.len(),
        1,
        "written under the chosen profile: {items:?}"
    );
    assert!(
        stored_items(&store, &runtime, Some("local"), "analytics.public.orders")
            .await
            .is_empty(),
        "nothing may leak onto the other profile"
    );

    // The same file against `local` — whose schema does not name `orders` —
    // resolves nothing: every item is reported unavailable and nothing lands.
    let (code, out, err) = run(
        ContractsCommand::Import {
            path: path.clone(),
            profile: None,
            preview: false,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_ne!(
        code, 0,
        "an import that lands nothing must exit non-zero: {out}{err}"
    );
    assert!(out.contains("unavailable 1"), "summary: {out}");
    assert!(
        out.contains("orders"),
        "the unavailable item is named: {out}"
    );
    assert!(
        stored_items(&store, &runtime, Some("local"), "analytics.public.orders")
            .await
            .is_empty(),
        "an unavailable item is never written"
    );

    drop(store);
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn conflicting_time_columns_require_decision() {
    let source_root = temp_root("conflict_source");
    let source_runtime = runtime_at(&source_root);
    let source_store = store_at(&source_root, &source_runtime).await;
    remember_active(
        &source_store,
        &source_runtime,
        ClaimKindArg::TimeColumn,
        "order_date",
        None,
    )
    .await;
    let export_path = source_root.join("context.json");
    let (code, _out, err) = run(
        ContractsCommand::Export {
            path: export_path.clone(),
            profile: None,
            overwrite: false,
        },
        &source_runtime,
        &source_store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(code, 0, "export failed: {err}");

    // The target profile holds a DIFFERENT active default time column for the
    // same object. Both claims cannot hold: the local decision stands, the
    // import is reported as a conflict, and nothing is written.
    let target_root = temp_root("conflict_target");
    let target_runtime = runtime_at(&target_root);
    let target_store = store_at(&target_root, &target_runtime).await;
    let identity = identity_for(&target_runtime, "local");
    target_store
        .upsert_schema(&identity, &orders_schema(&["orders"]))
        .await
        .unwrap();
    remember_active(
        &target_store,
        &target_runtime,
        ClaimKindArg::TimeColumn,
        "return_date",
        None,
    )
    .await;

    let (code, out, err) = run(
        ContractsCommand::Import {
            path: export_path,
            profile: None,
            preview: false,
        },
        &target_runtime,
        &target_store,
        RenderFormat::Text,
    )
    .await;
    assert_ne!(
        code, 0,
        "nothing landed, so the import exits non-zero: {out}{err}"
    );
    assert!(out.contains("conflicts 1"), "summary: {out}");
    assert!(out.contains("conflict"), "the conflict is reported: {out}");
    let items = stored_items(
        &target_store,
        &target_runtime,
        None,
        "analytics.public.orders",
    )
    .await;
    assert_eq!(items.len(), 1, "the import wrote nothing: {items:?}");
    assert_eq!(items[0].state, KnowledgeState::Active);
    assert!(
        matches!(
            &items[0].value,
            ClaimPayload::DefaultTimeColumn { column, .. } if column == "return_date"
        ),
        "the local claim stands: {:?}",
        items[0].value
    );
    assert!(
        out.contains(&items[0].id),
        "the conflict names the existing claim id: {out}"
    );

    drop(target_store);
    drop(source_store);
    let _ = fs::remove_dir_all(source_root);
    let _ = fs::remove_dir_all(target_root);
}

#[tokio::test]
async fn context_import_is_idempotent_and_atomic() {
    let root = temp_root("idempotent");
    let runtime = runtime_at(&root);
    let store = store_at(&root, &runtime).await;
    let identity = identity_for(&runtime, "local");
    store
        .upsert_schema(&identity, &orders_schema(&["orders"]))
        .await
        .unwrap();

    let items = vec![
        context_item(
            "analytics",
            "public",
            "orders",
            &ClaimPayload::table_description("One row per confirmed order.").unwrap(),
        ),
        context_item(
            "analytics",
            "public",
            "orders",
            &ClaimPayload::table_alias("customers").unwrap(),
        ),
    ];
    let path = root.join("context.json");
    write_context_file(&path, &context_document(items));

    let (code, out, _err) = run(
        ContractsCommand::Import {
            path: path.clone(),
            profile: None,
            preview: false,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(code, 0, "first import: {out}");
    assert!(out.contains("imported 2"), "{out}");
    let first = stored_items(&store, &runtime, None, "analytics.public.orders").await;
    assert_eq!(first.len(), 2);

    // Re-importing the same file is the no-op: identical items are skipped,
    // and the store holds exactly the same rows.
    let (code, out, _err) = run(
        ContractsCommand::Import {
            path: path.clone(),
            profile: None,
            preview: false,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(code, 0, "re-import: {out}");
    assert!(out.contains("skipped 2"), "identical items skip: {out}");
    assert_eq!(
        stored_items(&store, &runtime, None, "analytics.public.orders")
            .await
            .len(),
        2,
        "a skipped import writes nothing"
    );

    // Atomicity: a document that validates but carries one item the store's
    // admission gate refuses (an absolute path is machine state, not business
    // context) must write NOTHING — not even its valid sibling.
    let poisoned = vec![
        context_item(
            "analytics",
            "public",
            "orders",
            &ClaimPayload::table_description("See the notes at /home/alice/notes.txt.").unwrap(),
        ),
        context_item(
            "analytics",
            "public",
            "orders",
            &ClaimPayload::table_alias("never-lands").unwrap(),
        ),
    ];
    let poisoned_path = root.join("poisoned.json");
    write_context_file(&poisoned_path, &context_document(poisoned));
    let (code, out, err) = run(
        ContractsCommand::Import {
            path: poisoned_path,
            profile: None,
            preview: false,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_ne!(code, 0, "a refused batch exits non-zero: {out}{err}");
    let after = stored_items(&store, &runtime, None, "analytics.public.orders").await;
    assert_eq!(after.len(), 2, "the whole batch rolled back: {after:?}");
    assert!(
        !out.contains("never-lands") || out.contains("refused"),
        "no partial success may be reported: {out}"
    );

    drop(store);
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn memory_off_import_does_not_silently_enable_recall() {
    let root = temp_root("memory_off");
    // No [memory] section: the resolved mode is the default `off`.
    let runtime = runtime_at(&root);
    assert_eq!(runtime.resolved.memory.mode, MemoryMode::Off);
    let store = store_at(&root, &runtime).await;
    let identity = identity_for(&runtime, "local");
    store
        .upsert_schema(&identity, &orders_schema(&["orders"]))
        .await
        .unwrap();

    let items = vec![context_item(
        "analytics",
        "public",
        "orders",
        &ClaimPayload::table_description("One row per confirmed order.").unwrap(),
    )];
    let path = root.join("context.json");
    write_context_file(&path, &context_document(items));

    let (code, out, err) = run(
        ContractsCommand::Import {
            path,
            profile: None,
            preview: false,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(code, 0, "import with memory off still works: {out}{err}");
    assert!(out.contains("imported 1"), "{out}");
    assert!(
        out.contains("Memory is off, so these claims are not used until you enable it."),
        "the memory-off note: {out}"
    );
    // The claims landed pending, and recall stays off: nothing recalled.
    let items = stored_items(&store, &runtime, None, "analytics.public.orders").await;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].state, KnowledgeState::Pending);

    drop(store);
    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// dbt manifest import + preview
// ---------------------------------------------------------------------------

#[tokio::test]
async fn import_dbt_fixture_lands_pending_descriptions_and_relationship() {
    let root = temp_root("dbt_v12");
    let runtime = runtime_at(&root);
    let store = store_at(&root, &runtime).await;
    let identity = identity_for(&runtime, "local");
    store.upsert_schema(&identity, &dbt_schema()).await.unwrap();

    let manifest = root.join("manifest.json");
    fs::write(
        &manifest,
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/contracts/dbt/fixtures/v12_manifest.json"
        )),
    )
    .unwrap();

    let (code, out, err) = run(
        ContractsCommand::ImportDbt {
            manifest: manifest.clone(),
            profile: None,
            select: vec![],
            preview: false,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(code, 0, "dbt import failed: {out}{err}");
    // The fixture maps 9 items: 6 descriptions and 3 relationships.
    assert!(out.contains("imported 9"), "summary: {out}");
    assert!(
        out.contains("Imported items are pending review: saya contracts queue"),
        "the queue pointer: {out}"
    );

    // Every landed row is Pending with dbt provenance.
    for (name, qualified) in [
        ("public_orders", "analytics.core.public_orders"),
        ("customers", "analytics.core.customers"),
        ("hidden", "analytics.core.hidden"),
        ("raw_events", "raw.jaffle.raw_events"),
    ] {
        let items = stored_items(&store, &runtime, None, qualified).await;
        assert!(!items.is_empty(), "{name} has no imported items");
        assert!(
            items
                .iter()
                .all(|item| item.state == KnowledgeState::Pending),
            "{name} items must be pending: {items:?}"
        );
        assert!(
            items
                .iter()
                .all(|item| item.source == ClaimOrigin::TeamFile),
            "{name} items carry the team-file origin: {items:?}"
        );
    }
    // The relationships landed as join rules — the one relationship-shaped
    // slot the store can file — with the paired columns as the join keys.
    let orders = stored_items(&store, &runtime, None, "analytics.core.public_orders").await;
    let joins: Vec<_> = orders
        .iter()
        .filter(|item| item.slot == KnowledgeSlot::RelationJoinRule)
        .collect();
    assert_eq!(
        joins.len(),
        2,
        "orders carries two imported relationships: {orders:?}"
    );
    assert!(
        joins.iter().any(|item| matches!(
            &item.value,
            ClaimPayload::JoinRule { target, local_columns, target_columns, .. }
                if target == "analytics.core.customers"
                    && local_columns == &["customer_id".to_string()]
                    && target_columns == &["id".to_string()]
        )),
        "the ref relationship filed as a keyed join rule: {joins:?}"
    );
    assert!(
        joins.iter().any(|item| matches!(
            &item.value,
            ClaimPayload::JoinRule { target, local_columns, target_columns, .. }
                if target == "raw.jaffle.raw_events"
                    && local_columns == &["event_id".to_string()]
                    && target_columns == &["event_key".to_string()]
        )),
        "the source relationship filed as a keyed join rule: {joins:?}"
    );

    // Re-importing the same manifest is idempotent.
    let (code, out, _err) = run(
        ContractsCommand::ImportDbt {
            manifest: manifest.clone(),
            profile: None,
            select: vec![],
            preview: false,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(code, 0, "re-import: {out}");
    assert!(out.contains("skipped 9"), "identical items skip: {out}");

    // A --select narrows the mapping: the glob matches node names, so
    // `orders` selects the orders model alone — its three description items —
    // while its relationships' targets (customers, events) are unselected and
    // reported as skipped.
    let fresh_root = temp_root("dbt_select");
    let fresh_runtime = runtime_at(&fresh_root);
    let fresh_store = store_at(&fresh_root, &fresh_runtime).await;
    let fresh_identity = identity_for(&fresh_runtime, "local");
    fresh_store
        .upsert_schema(&fresh_identity, &dbt_schema())
        .await
        .unwrap();
    let (code, out, err) = run(
        ContractsCommand::ImportDbt {
            manifest: manifest.clone(),
            profile: None,
            select: vec!["orders".into()],
            preview: false,
        },
        &fresh_runtime,
        &fresh_store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(code, 0, "select import failed: {out}{err}");
    assert!(out.contains("imported 3"), "only orders' own items: {out}");
    assert!(
        out.contains("unavailable 2"),
        "the unselected targets skip: {out}"
    );
    assert!(
        out.contains("unresolved_target"),
        "the skip reason is named: {out}"
    );

    drop(store);
    drop(fresh_store);
    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(fresh_root);
}

#[tokio::test]
async fn context_import_preview_writes_nothing() {
    let root = temp_root("preview");
    let runtime = runtime_at(&root);
    let store = store_at(&root, &runtime).await;
    let identity = identity_for(&runtime, "local");
    store
        .upsert_schema(&identity, &orders_schema(&["orders"]))
        .await
        .unwrap();

    let items = vec![context_item(
        "analytics",
        "public",
        "orders",
        &ClaimPayload::table_description("One row per confirmed order.").unwrap(),
    )];
    let path = root.join("context.json");
    write_context_file(&path, &context_document(items));

    let (code, out, err) = run(
        ContractsCommand::Import {
            path,
            profile: None,
            preview: true,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(code, 0, "preview failed: {out}{err}");
    assert!(out.contains("would import 1"), "the preview summary: {out}");
    assert!(
        out.contains("Preview only: nothing was written."),
        "the preview posture: {out}"
    );
    assert!(
        stored_items(&store, &runtime, None, "analytics.public.orders")
            .await
            .is_empty(),
        "a preview writes nothing"
    );

    // The dbt preview behaves the same.
    let manifest = root.join("manifest.json");
    fs::write(
        &manifest,
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/contracts/dbt/fixtures/v12_manifest.json"
        )),
    )
    .unwrap();
    let (code, out, err) = run(
        ContractsCommand::ImportDbt {
            manifest,
            profile: None,
            select: vec![],
            preview: true,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(code, 0, "dbt preview failed: {out}{err}");
    assert!(
        out.contains("would import"),
        "the dbt preview summary: {out}"
    );
    assert!(
        out.contains("Preview only: nothing was written."),
        "the dbt preview posture: {out}"
    );
    assert!(
        stored_items(&store, &runtime, None, "analytics.core.public_orders")
            .await
            .is_empty(),
        "a dbt preview writes nothing"
    );

    drop(store);
    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// slash parity: the same enum, not a second parser
// ---------------------------------------------------------------------------

#[test]
fn slash_export_import_import_dbt_parse_matches_headless() {
    for (line, expected) in [
        (
            "/contracts export /tmp/context.json",
            ContractsCommand::Export {
                path: "/tmp/context.json".into(),
                profile: None,
                overwrite: false,
            },
        ),
        (
            "/contracts export /tmp/context.json --overwrite",
            ContractsCommand::Export {
                path: "/tmp/context.json".into(),
                profile: None,
                overwrite: true,
            },
        ),
        (
            "/contracts import /tmp/context.json --preview",
            ContractsCommand::Import {
                path: "/tmp/context.json".into(),
                profile: None,
                preview: true,
            },
        ),
        (
            "/contracts import-dbt /tmp/manifest.json",
            ContractsCommand::ImportDbt {
                manifest: "/tmp/manifest.json".into(),
                profile: None,
                select: vec![],
                preview: false,
            },
        ),
        (
            "/contracts import-dbt /tmp/manifest.json --select model.jaffle.* --preview",
            ContractsCommand::ImportDbt {
                manifest: "/tmp/manifest.json".into(),
                profile: None,
                select: vec!["model.jaffle.*".into()],
                preview: true,
            },
        ),
    ] {
        let parsed = match parse_slash_command(line) {
            Ok(Some(SlashCommand::Contracts(command))) => command,
            other => panic!("{line:?} must parse to a contracts command, got {other:?}"),
        };
        assert_eq!(parsed, expected, "{line:?} must equal the headless command");
    }

    // Missing paths and unknown flags are usage errors, never silent ops.
    for line in [
        "/contracts export",
        "/contracts import --preview",
        "/contracts import-dbt",
        "/contracts export /tmp/a.json --bogus",
        "/contracts import-dbt /tmp/m.json --select",
    ] {
        assert!(
            parse_slash_command(line).is_err(),
            "{line:?} must be a usage error"
        );
    }
}
