//! knowledge_items store security — the guarantee the legacy `contract_security`
//! suite carried, re-homed on the new write path.
//!
//! `knowledge_items` is now the sole place a fact SAYA knows is persisted. The
//! production admission gate (`knowledge_items/writes.rs`) refuses a value that
//! structurally resembles a secret — PEM armour, a credential header, an absolute
//! filesystem path, a SQL statement, a URL with inline credentials — rather than
//! scrubbing and storing it. That gate survived the legacy excision, but nothing
//! tested it. This suite does, for both persisted channels the write path now has:
//! `value_json` *and* `schema_binding_json` (the binding is a second persisted
//! channel, so it gets the same gate — see the comment at `writes.rs:37`).
//!
//! Three guarantees, mirroring the deleted suite:
//! 1. Each structurally-recognisable secret shape is **refused at admission**.
//! 2. It **never reaches the database bytes**, verified by scanning the SQLite
//!    file and its `-wal` and `-shm` sidecars after the write.
//! 3. The store **cannot** structurally recognise a bare opaque token —
//!    `SENTINELPASSWORD`, a single result cell — because nothing distinguishes it
//!    from a legitimate business term. That limit is documented and tested, not
//!    quietly dropped: rejecting it would reject `hunter2` and `SHIPPED` alike.

use saya_store::{
    KnowledgeItemRequest, KnowledgeItemStore, KnowledgeStoreError, SqliteStateStore, StoreError,
    state_sidecar_path,
};
use saya_types::{
    ClaimOrigin, ClaimPayload, DatabaseObjectKind, DatabaseObjectRef, KnowledgeSlot,
    KnowledgeState, ProfileIdentity, SchemaFingerprint,
};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

/// Sentinels the store recognises by *structure*. Every one must be refused at
/// admission and must never reach the database bytes — in either persisted
/// channel (value or schema binding).
const STRUCTURAL_SENTINELS: &[&str] = &[
    "postgres://user:SENTINELPASSWORD@host/db",
    "-----BEGIN PRIVATE KEY-----",
    "SELECT SENTINELCOLUMN FROM orders",
    "x-api-key: SENTINELTOKEN",
    "/Users/SENTINELUSER/secret/project",
];

/// Sentinels the store *cannot* recognise, and the reason it cannot.
///
/// `SENTINELPASSWORD` is a bare opaque token and `SENTINELROWVALUE` is a single
/// cell copied out of a result set. Neither has any structure separating it from
/// a legitimate business term — a product code, a status name, a tier label. A
/// string inspector that rejected them would reject `hunter2` and `SHIPPED`
/// alike and make the feature unusable. They are kept out by construction (the
/// typed payload allow-list, and the learning envelope never showing result rows
/// to the extractor), not by inspection. This constant exists so that boundary is
/// written down and tested, not assumed.
const UNDETECTABLE_SENTINELS: &[&str] = &["SENTINELPASSWORD", "SENTINELROWVALUE"];

// ---------------------------------------------------------------------------
// Shared helpers (process-unique temp dirs; no cross-test sharing)
// ---------------------------------------------------------------------------

fn temp_root(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("saya-ksec-{label}-{}-{stamp}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    root
}

fn profile() -> ProfileIdentity {
    ProfileIdentity::parse(&format!("p-{}", "a".repeat(64))).unwrap()
}

fn object(name: &str) -> DatabaseObjectRef {
    DatabaseObjectRef::new(
        profile(),
        "catalog",
        "public",
        name,
        DatabaseObjectKind::Table,
    )
    .unwrap()
}

fn fingerprint() -> SchemaFingerprint {
    SchemaFingerprint::from_parts(1, &"f".repeat(64)).unwrap()
}

/// A clean schema binding for the ordinary write path. The sentinel-carrying
/// requests override this channel explicitly.
fn clean_binding() -> String {
    r#"{"columns":["user_id"]}"#.to_owned()
}

/// Build a `table.grain` request, the slot the existing suite uses. `grain` is
/// single-valued, so one refusal leaves at most the schema row behind.
fn grain_request(value: ClaimPayload, binding: String) -> KnowledgeItemRequest {
    KnowledgeItemRequest {
        object: object("orders"),
        slot: KnowledgeSlot::TableGrain,
        value,
        source: ClaimOrigin::UserExplicit,
        state: KnowledgeState::Active,
        schema_binding_json: binding,
        fingerprint: fingerprint(),
    }
}

/// Concatenate the raw bytes of the database and any populated sidecars. WAL mode
/// keeps freshly-written rows in `-wal` until checkpoint, so the sentinel may live
/// only there — scanning all three is what makes the check honest.
fn db_bytes(db: &Path) -> Vec<u8> {
    let mut out = fs::read(db).unwrap_or_default();
    for suffix in ["-wal", "-shm"] {
        let sidecar = state_sidecar_path(db, suffix);
        if sidecar.exists() {
            out.extend(fs::read(&sidecar).unwrap_or_default());
        }
    }
    out
}

/// Assert `bytes` contain none of the structural sentinels. On a hit, panic loudly
/// with the exact sentinel so the failure names the leak.
fn assert_no_sentinels(label: &str, bytes: &[u8]) {
    for sentinel in STRUCTURAL_SENTINELS {
        assert!(
            !window_contains(bytes, sentinel.as_bytes()),
            "LEAK: sentinel `{sentinel}` found in {label} bytes"
        );
    }
}

/// Naive byte-substring search (SQLite stores text as UTF-8).
fn window_contains(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || needle.len() > haystack.len() {
        return false;
    }
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// Stable, filesystem-safe label derived from a sentinel for the temp dir.
/// Sentinels contain `/`, `:`, `@`, and spaces — naively joining them would make
/// nested directories.
fn hash_label(sentinel: &str) -> String {
    let mut acc: u64 = 0xCBF29CE484222325;
    for &byte in sentinel.as_bytes() {
        acc ^= byte as u64;
        acc = acc.wrapping_mul(0x100000001B3);
    }
    format!("{acc:016x}")
}

// ---------------------------------------------------------------------------
// §1  Structural secrets in the value are refused and never reach the bytes
// ---------------------------------------------------------------------------

/// For each structural sentinel, attempt to store it as a `table.grain` value.
/// Every one must reject with `Store(Invalid)` and leave clean bytes — the
/// admission check runs before any INSERT, so a refused write stores nothing.
#[tokio::test]
async fn structural_secret_in_the_value_is_refused_and_never_reaches_the_bytes() {
    for &sentinel in STRUCTURAL_SENTINELS {
        let root = temp_root(&format!("val-{}", hash_label(sentinel)));
        let db = root.join("state.sqlite3");
        let store = SqliteStateStore::new(&db);

        let result = store
            .put_knowledge_item(grain_request(
                ClaimPayload::table_grain(sentinel).unwrap(),
                clean_binding(),
            ))
            .await;
        let bytes = db_bytes(&db);
        store.close().await;
        let _ = fs::remove_dir_all(root);

        assert_eq!(
            result,
            Err(KnowledgeStoreError::Store(StoreError::Invalid)),
            "`{sentinel}` was admitted; the admission gate must refuse it",
        );
        assert!(
            !window_contains(&bytes, sentinel.as_bytes()),
            "LEAK: `{sentinel}` reached the database bytes via the value",
        );
    }
}

// ---------------------------------------------------------------------------
// §2  Structural secrets in the schema binding are refused too
// ---------------------------------------------------------------------------
//
// The binding is a second persisted channel the legacy suite did not have. The
// production gate checks it with the same `redact` + `admission::check` pair
// (`writes.rs:44-47`), so a sentinel smuggled in through `schema_binding_json`
// must be refused for the same reason — and must not reach the bytes.

/// For each structural sentinel, store a clean value but carry the sentinel in
/// `schema_binding_json`. The binding gate must refuse it and store nothing.
#[tokio::test]
async fn structural_secret_in_the_schema_binding_is_refused_and_never_reaches_the_bytes() {
    for &sentinel in STRUCTURAL_SENTINELS {
        let root = temp_root(&format!("bind-{}", hash_label(sentinel)));
        let db = root.join("state.sqlite3");
        let store = SqliteStateStore::new(&db);

        let result = store
            .put_knowledge_item(grain_request(
                ClaimPayload::table_grain("one row per order").unwrap(),
                sentinel.to_owned(),
            ))
            .await;
        let bytes = db_bytes(&db);
        store.close().await;
        let _ = fs::remove_dir_all(root);

        assert_eq!(
            result,
            Err(KnowledgeStoreError::Store(StoreError::Invalid)),
            "`{sentinel}` in the schema binding was admitted; the binding gate must refuse it",
        );
        assert!(
            !window_contains(&bytes, sentinel.as_bytes()),
            "LEAK: `{sentinel}` reached the database bytes via the schema binding",
        );
    }
}

// ---------------------------------------------------------------------------
// §3  A clean full lifecycle leaves no sentinels in db or sidecars
// ---------------------------------------------------------------------------

/// Drive a full lifecycle on clean text — put, revalidate, state transition, a
/// second multi-valued item, delete — then scan db + -wal + -shm before close
/// (WAL still populated) and again after. No sentinel may appear in any file.
/// This proves the ordinary write path never smuggles one in through the back
/// door, across every write method the repository exposes.
#[tokio::test]
async fn full_lifecycle_leaves_no_sentinels_in_db_or_sidecars() {
    let root = temp_root("lifecycle");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let obj = object("orders");

    // put (insert_or_replace) — the single-valued grain slot.
    store
        .put_knowledge_item(grain_request(
            ClaimPayload::table_grain("one row per order").unwrap(),
            clean_binding(),
        ))
        .await
        .unwrap();
    let id = store.knowledge_for_object(&obj).await.unwrap()[0]
        .id
        .clone();

    // revalidate (revalidate_item) — a clean new binding + fingerprint version.
    let (fp2, binding2) = (
        SchemaFingerprint::from_parts(2, &"f".repeat(64)).unwrap(),
        r#"{"columns":["order_id"]}"#.to_owned(),
    );
    store
        .revalidate_knowledge_item(&id, fp2, binding2)
        .await
        .unwrap();

    // state transition (update_state) — no user text on this path, but exercise it.
    store
        .update_knowledge_item_state(&id, KnowledgeState::Dismissed)
        .await
        .unwrap();

    // a second item on a multi-valued slot (append path).
    store
        .put_knowledge_item(KnowledgeItemRequest {
            object: obj.clone(),
            slot: KnowledgeSlot::TableAlias,
            value: ClaimPayload::table_alias("orders").unwrap(),
            source: ClaimOrigin::UserExplicit,
            state: KnowledgeState::Active,
            schema_binding_json: clean_binding(),
            fingerprint: fingerprint(),
        })
        .await
        .unwrap();

    // delete (delete_item) the first item.
    store.delete_knowledge_item(&id).await.unwrap();

    // Before close: WAL still holds uncheckpointed pages.
    assert_no_sentinels("pre-close (db+wal+shm)", &db_bytes(&db));
    store.close().await;
    // After close: sidecars may have been checkpointed/removed; scan what remains.
    assert_no_sentinels("post-close (db+wal+shm)", &db_bytes(&db));
    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// §4  The documented limit: opaque tokens are stored, not refused
// ---------------------------------------------------------------------------

/// A bare opaque token and a single result-row cell have no structure separating
/// them from a legitimate business term. The store admits them — and they DO
/// reach the bytes. This test is written to fail if that limit ever changes
/// silently in either direction: if the store learned to refuse this class, the
/// `is_ok` assertion fails (move the sentinel into `STRUCTURAL_SENTINELS`); if it
/// learned to scrub-and-store, the byte assertion fails (a silent partial fix is
/// the worst outcome). Keeping secrets and result cells out is the job of the
/// typed payload allow-list and the learning envelope, not of string inspection.
#[tokio::test]
async fn opaque_sentinels_are_stored_because_nothing_distinguishes_them() {
    for &sentinel in UNDETECTABLE_SENTINELS {
        let root = temp_root(&format!("opaque-{}", hash_label(sentinel)));
        let db = root.join("state.sqlite3");
        let store = SqliteStateStore::new(&db);

        let result = store
            .put_knowledge_item(grain_request(
                ClaimPayload::table_grain(sentinel).unwrap(),
                clean_binding(),
            ))
            .await;
        let bytes = db_bytes(&db);
        store.close().await;
        let _ = fs::remove_dir_all(root);

        assert!(
            result.is_ok(),
            "`{sentinel}` was refused — if the store learned to detect this class, \
             move it into STRUCTURAL_SENTINELS and delete it from here",
        );
        assert!(
            window_contains(&bytes, sentinel.as_bytes()),
            "`{sentinel}` was admitted but did not reach the bytes — a silent scrub \
             is worse than an honest admit; investigate before weakening this",
        );
    }
}
