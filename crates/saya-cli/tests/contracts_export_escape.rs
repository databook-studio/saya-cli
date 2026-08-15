//! P1 regression: `saya contracts export` must not escape its destination.
//!
//! Export builds a filename from a stored object's identity. A database
//! identifier is attacker-influenceable (a catalog/schema/object name), so the
//! filename must be *derived* from an encoded identity, not copied from the
//! raw name, and the write must be contained inside a canonicalised
//! destination. These tests plant hostile identifiers through the public API
//! the way the byte-scan tests do — seed a confirmed claim with the hostile
//! name, export, and assert both the typed failure and that nothing outside the
//! destination changed.
//!
//! They must fail before the fix: against the unfixed `destination.join(format!(
//! "{}.toml", qualified_name()))` scheme, an absolute catalog name makes `join`
//! discard the destination and the file lands outside it (`--force` then
//! clobbers it); a `..`-with-`/` traversal in the object name resolves the
//! parent outside the destination; and the `Instant::now().elapsed()` temp name
//! collides between concurrent exports.

use saya_cli::{
    ContractsCommand, RenderFormat, RuntimeConfig, capture_output_start, capture_output_take,
    load_with_sources, profile_identity, run_contracts,
};
use saya_store::{
    ContractStore, ProposeClaim, ProposeOutcome, SchemaStore, SqliteStateStore, object_id,
};
use saya_types::{
    ClaimOrigin, ClaimStatus, DatabaseObjectKind, DatabaseObjectRef, ProfileIdentity,
};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

// ---------------------------------------------------------------------------
// harness — mirrors tests/contracts_import_export.rs
// ---------------------------------------------------------------------------

fn temp_root(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "saya-p1-export-{label}-{}-{stamp}",
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
    let (runtime, _) = runtime_at(root);
    let identity = identity_for(&runtime, "local");
    store
        .upsert_schema(&identity, &saya_types::SchemaTree::default())
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

fn unobserved_fingerprint() -> saya_types::SchemaFingerprint {
    saya_types::SchemaFingerprint::from_parts(saya_types::FINGERPRINT_VERSION, &"0".repeat(64))
        .unwrap()
}

/// Build a `DatabaseObjectRef` with a hostile name in one of the three slots.
/// `validate_name` only rejects empty/long/control-char names, so `/`, `\` and
/// `..` pass straight through to the store — that is the bug.
fn object_ref_with(
    runtime: &RuntimeConfig,
    catalog: &str,
    schema: &str,
    object: &str,
) -> DatabaseObjectRef {
    DatabaseObjectRef::new(
        profile_for(runtime, "local"),
        catalog,
        schema,
        object,
        DatabaseObjectKind::Table,
    )
    .unwrap()
}

/// Seed a confirmed claim for the given hostile object, straight through the
/// store the way a real `remember` would. Confirmed so export picks it up.
async fn seed_confirmed(
    store: &SqliteStateStore,
    _runtime: &RuntimeConfig,
    object: DatabaseObjectRef,
) {
    let request = ProposeClaim {
        object,
        fingerprint: unobserved_fingerprint(),
        payload: saya_types::ClaimPayload::table_alias("customers").unwrap(),
        origin: ClaimOrigin::UserExplicit,
        initial_status: ClaimStatus::Confirmed,
        evidence: None,
        referenced_columns: Vec::new(),
    };
    assert!(matches!(
        store.propose_claim(request).await.unwrap(),
        ProposeOutcome::Stored(_)
    ));
}

/// The filename export must use: the encoded object identity, which is
/// `o-<64 hex>` and filesystem-safe by construction — no `/`, `..`, or
/// absolute prefix can survive it.
fn expected_filename(object: &DatabaseObjectRef) -> String {
    format!("{}.toml", object_id(object))
}

/// A plant-the-flag sentinel file outside the destination. If it is touched
/// after the export, the write escaped.
fn plant_flag(outside_root: &Path, name: &str) -> PathBuf {
    let flag = outside_root.join(name);
    fs::write(&flag, b"SENTINEL").unwrap();
    flag
}

/// Snapshot every regular file under `dir` (recursively) as a sorted set of
/// relative paths with their bytes, so we can prove what changed outside the
/// destination.
fn snapshot(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in fs::read_dir(&d).unwrap().flatten() {
            let p = entry.path();
            let ft = entry.file_type().unwrap();
            if ft.is_dir() {
                stack.push(p);
            } else if ft.is_file() {
                let rel = p.strip_prefix(dir).unwrap().to_path_buf();
                out.push((rel, fs::read(&p).unwrap()));
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

async fn run_export(
    runtime: &RuntimeConfig,
    store: &SqliteStateStore,
    dest: &Path,
    force: bool,
) -> (i32, String, String) {
    run(
        ContractsCommand::Export {
            destination: dest.to_path_buf(),
            profile: None,
            force,
        },
        runtime,
        store,
        RenderFormat::Text,
    )
    .await
}

// ---------------------------------------------------------------------------
// 1. An object name containing `/` does not write outside the destination.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn slash_in_object_name_stays_inside_destination() {
    let root = temp_root("slash-obj");
    let (runtime, _) = runtime_at(&root);
    let store = store_at(&root).await;
    let object = object_ref_with(&runtime, "analytics", "public", "a/b");
    seed_confirmed(&store, &runtime, object.clone()).await;

    let dest = root.join("out");
    fs::create_dir_all(&dest).unwrap();
    let (code, _out, err) = run_export(&runtime, &store, &dest, false).await;
    assert_eq!(code, 0, "export failed: {err}");

    // The file exists under the destination, named by the encoded identity.
    let file = dest.join(expected_filename(&object));
    assert!(file.exists(), "expected file at {file:?}");
    // No subdirectory was created inside the destination — a flat filename
    // has no separators, so `a/b` cannot become a nested path.
    assert!(
        !dest.join("analytics.public.a").exists(),
        "a `/` in the object name created a nested directory under the destination"
    );
    // And the body carries the qualified name as *data*, inside the file.
    let body = fs::read_to_string(&file).unwrap();
    assert!(
        body.contains("analytics.public.a/b"),
        "qualified name not in body: {body}"
    );

    let _ = fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------
// 2. An absolute object name (`/tmp/pwned`) does not write outside the dest.
//    The headline exploit: with the old scheme `join` discards the destination
//    when the joined name is absolute, so the file lands at the absolute path.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn absolute_catalog_name_does_not_escape() {
    let root = temp_root("abs-catalog");
    let (runtime, _) = runtime_at(&root);
    let store = store_at(&root).await;

    // A catalog name that is an absolute path under /tmp. This is the shape
    // that, pre-fix, made `destination.join(...)` drop the destination.
    let outside_root = root.join("outside");
    fs::create_dir_all(&outside_root).unwrap();
    let target_dir = outside_root.join("tmp");
    fs::create_dir_all(&target_dir).unwrap();
    // The absolute path the hostile catalog name would resolve to, relative
    // to the temp root, so the exploit would land here pre-fix:
    let would_escape = target_dir.join("pwned.public.orders.toml");
    // Build an object whose catalog name, when joined, produces `would_escape`.
    // We use the absolute path string of `target_dir/pwned` as the catalog.
    let hostile_catalog = target_dir.join("pwned").to_string_lossy().to_string();
    let object = object_ref_with(&runtime, &hostile_catalog, "public", "orders");
    seed_confirmed(&store, &runtime, object.clone()).await;

    let dest = root.join("out");
    fs::create_dir_all(&dest).unwrap();
    // Plant the flag at the would-escape location so we can detect a clobber.
    let flag = plant_flag(&target_dir, "pwned.public.orders.toml");

    let (code, _out, err) = run_export(&runtime, &store, &dest, true).await;
    assert_eq!(code, 0, "export failed: {err}");

    // The flag outside the destination is untouched — no escape, no clobber.
    assert_eq!(
        fs::read(&flag).unwrap(),
        b"SENTINEL",
        "the write escaped the destination and clobbered an outside file"
    );
    assert!(
        !would_escape.exists() || flag == would_escape,
        "a file was created outside the destination at {would_escape:?}"
    );
    // The real file is inside the destination, named by the encoded identity.
    assert!(dest.join(expected_filename(&object)).exists());

    let _ = fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------
// 3. A name containing `..` components does not escape.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn dotdot_traversal_does_not_escape() {
    let root = temp_root("dotdot");
    let (runtime, _) = runtime_at(&root);
    let store = store_at(&root).await;

    // `a/../../pwned` in the object slot: pre-fix, `qualified_name` yields
    // `analytics.public.a/../../pwned`, and the lexical `..` resolves the
    // parent outside the destination.
    let object = object_ref_with(&runtime, "analytics", "public", "a/../../pwned");
    seed_confirmed(&store, &runtime, object.clone()).await;

    let dest = root.join("out");
    fs::create_dir_all(&dest).unwrap();
    // A flag one level above the destination, where the `..` would land.
    let flag = plant_flag(&root, "pwned.toml");

    let (code, _out, err) = run_export(&runtime, &store, &dest, true).await;
    assert_eq!(code, 0, "export failed: {err}");

    // The flag above the destination is untouched.
    assert_eq!(
        fs::read(&flag).unwrap(),
        b"SENTINEL",
        "a `..` traversal escaped above the destination"
    );
    // The file is inside the destination.
    assert!(dest.join(expected_filename(&object)).exists());

    let _ = fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------
// 4. A destination containing a symlink to elsewhere does not let the write
//    land outside the canonical destination.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn symlinked_destination_does_not_redirect_outside() {
    let root = temp_root("symlink-dest");
    let (runtime, _) = runtime_at(&root);
    let store = store_at(&root).await;
    let object = object_ref_with(&runtime, "analytics", "public", "orders");
    seed_confirmed(&store, &runtime, object.clone()).await;

    // A "real" destination dir, and a symlink the user names that points at it.
    let real_dest = root.join("real-out");
    fs::create_dir_all(&real_dest).unwrap();
    let symlink_dest = root.join("link-out");
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        symlink(&real_dest, &symlink_dest).unwrap();
    }
    #[cfg(not(unix))]
    let symlink_dest = real_dest.clone();

    // A hostile sibling: a symlink *named* like the target file, pointing
    // outside. With `--force` the write must not follow it to clobber the
    // outside target — it must land at the canonical destination.
    let outside = root.join("outside-target");
    fs::create_dir_all(&outside).unwrap();
    let outside_flag = plant_flag(&outside, "passwd");
    let trap_name = expected_filename(&object);
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        // Place the trap symlink inside the real destination, named like the
        // file export would write.
        symlink(&outside_flag, real_dest.join(&trap_name)).unwrap();
    }

    let (code, _out, err) = run_export(&runtime, &store, &symlink_dest, true).await;
    assert_eq!(code, 0, "export failed: {err}");

    // The outside target is untouched — the write did not follow the trap.
    assert_eq!(
        fs::read(&outside_flag).unwrap(),
        b"SENTINEL",
        "the write followed a symlink trap and clobbered an outside file"
    );
    // The canonical destination holds a real regular file with the body.
    let real_file = real_dest.join(&trap_name);
    assert!(
        real_file.exists(),
        "file missing at canonical dest: {real_file:?}"
    );
    let meta = fs::symlink_metadata(&real_file).unwrap();
    assert!(meta.is_file(), "target is not a regular file: {meta:?}");
    let body = fs::read_to_string(&real_file).unwrap();
    assert!(
        body.contains("analytics.public.orders"),
        "body missing object: {body}"
    );

    let _ = fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------
// 5. Two concurrent exports to the same destination do not collide or
//    interleave into one file.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn concurrent_exports_do_not_collide_or_interleave() {
    let root = temp_root("concurrent");
    let (runtime, _) = runtime_at(&root);
    let store = store_at(&root).await;
    let object = object_ref_with(&runtime, "analytics", "public", "orders");
    seed_confirmed(&store, &runtime, object.clone()).await;

    let dest = root.join("out");
    fs::create_dir_all(&dest).unwrap();

    // Run two exports concurrently with `--force`. Each must write its own
    // complete temp file; the destination is atomically one or the other,
    // never a byte-interleaved mix. The thread-local output-capture seam is
    // per-thread, so the two exports run on two OS threads (each with its own
    // current-thread runtime and capture buffer) to keep their `emit` calls
    // from racing one buffer. The store is `Arc`-shared, so both runtimes
    // drive the same pool. We assert on disk state, not the captured text.
    use std::sync::Arc;
    let rt = Arc::new(runtime);
    let st = Arc::new(store);
    let dest_arc = Arc::new(dest.clone());
    let run_one = move |rt: Arc<RuntimeConfig>, st: Arc<SqliteStateStore>, dest: Arc<PathBuf>| {
        std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async move { run_export(&rt, &st, &dest, true).await })
        })
        .join()
        .unwrap()
    };
    let h1 = std::thread::spawn({
        let (rt, st, dest_arc) = (rt.clone(), st.clone(), dest_arc.clone());
        move || run_one(rt, st, dest_arc)
    });
    let h2 = std::thread::spawn({
        let (rt, st, dest_arc) = (rt.clone(), st.clone(), dest_arc.clone());
        move || run_one(rt, st, dest_arc)
    });
    let (c1, _o1, e1) = h1.join().unwrap();
    let (c2, _o2, e2) = h2.join().unwrap();
    assert_eq!(c1, 0, "export 1 failed: {e1}");
    assert_eq!(c2, 0, "export 2 failed: {e2}");

    // Exactly one destination file exists, named by the encoded identity.
    let file = dest.join(expected_filename(&object));
    assert!(file.exists(), "destination file missing: {file:?}");
    // No leftover temp files in the destination.
    let leftovers: Vec<_> = fs::read_dir(&dest)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains("saya-export"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "temp files left behind: {leftovers:?}"
    );

    // The destination file is well-formed, not a byte-interleaved mix: it
    // parses as the discovered shape and carries exactly one alias claim.
    let body = fs::read_to_string(&file).unwrap();
    assert!(
        body.contains("version = 1"),
        "interleaved or truncated body: {body}"
    );
    assert!(
        body.contains("kind = \"alias\""),
        "body lost the claim: {body}"
    );
    assert_eq!(
        body.matches("kind = \"alias\"").count(),
        1,
        "body has more than one claim (interleaved?): {body}"
    );

    let _ = fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------
// 6. Without `--force`, an existing destination file is never overwritten —
//    including when it appears between the check and the write.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn no_force_does_not_clobber_existing_file() {
    let root = temp_root("no-clobber");
    let (runtime, _) = runtime_at(&root);
    let store = store_at(&root).await;
    let object = object_ref_with(&runtime, "analytics", "public", "orders");
    seed_confirmed(&store, &runtime, object.clone()).await;

    let dest = root.join("out");
    fs::create_dir_all(&dest).unwrap();
    let file = dest.join(expected_filename(&object));
    // Pre-existing content the export must NOT overwrite.
    fs::write(&file, "PRE-EXISTING\n").unwrap();

    let (code, _out, err) = run_export(&runtime, &store, &dest, false).await;
    // Without --force, the existing file is a typed error, not a silent write.
    assert_ne!(
        code, 0,
        "export overwrote an existing file without --force: {err}"
    );

    // The pre-existing content is intact.
    assert_eq!(
        fs::read_to_string(&file).unwrap(),
        "PRE-EXISTING\n",
        "the existing file was overwritten without --force"
    );
    // No temp file was left behind.
    let leftovers: Vec<_> = fs::read_dir(&dest)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains("saya-export"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "temp file left after a refused write: {leftovers:?}"
    );

    let _ = fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------
// 7. With `--force`, the overwrite still lands inside the destination.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn force_overwrite_lands_inside_destination() {
    let root = temp_root("force-inside");
    let (runtime, _) = runtime_at(&root);
    let store = store_at(&root).await;
    let object = object_ref_with(&runtime, "analytics", "public", "orders");
    seed_confirmed(&store, &runtime, object.clone()).await;

    let dest = root.join("out");
    fs::create_dir_all(&dest).unwrap();
    let file = dest.join(expected_filename(&object));
    fs::write(&file, "OLD\n").unwrap();

    let (code, _out, err) = run_export(&runtime, &store, &dest, true).await;
    assert_eq!(code, 0, "export failed: {err}");

    // The file inside the destination was replaced, and is inside it.
    let body = fs::read_to_string(&file).unwrap();
    assert_ne!(body, "OLD\n", "--force did not overwrite");
    assert!(
        body.contains("analytics.public.orders"),
        "body missing object: {body}"
    );

    // Nothing outside the destination changed: the dest tree is exactly one
    // file (the encoded-identity name) plus nothing else.
    let snap = snapshot(&dest);
    assert_eq!(
        snap.len(),
        1,
        "more than one file in the destination: {snap:?}"
    );
    assert_eq!(snap[0].0, Path::new(&expected_filename(&object)));

    let _ = fs::remove_dir_all(&root);
}
