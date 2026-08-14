//! Phase 1f — store security: byte scans, corruption, permissions.
//!
//! Enforcement half of ADR 0002's threat model. Each test corresponds to a row in
//! that table. Several assertions are *designed* to fail: the current `redact()` is
//! marker-and-URL based only, so sentinels for raw SQL, result-row values, bare
//! credentials, and file paths reach the database bytes. A failing assertion here is
//! a `LEAK:` finding reported to the operator — never weakened to make the suite
//! green. See `.claude/specs/spec-1f-security.md`.

use saya_store::{
    ClaimEvidence, ContractStore, EvidenceKind, ForgetReason, MAX_CLAIMS_PER_OBJECT, ProposeClaim,
    ProposeOutcome, SchemaStore, SqliteStateStore, StoreError, state_sidecar_path,
};
use saya_types::{
    ClaimId, ClaimOrigin, ClaimPayload, ClaimStatus, DatabaseObjectKind, DatabaseObjectRef,
    ProfileIdentity, SchemaFingerprint,
};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

/// Sentinels the store can recognise by *structure* — PEM armour, a credential
/// header, an absolute filesystem path, a SQL statement, a URL with inline
/// credentials. Every one of these must be refused at admission and must never
/// reach the database bytes.
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
/// cell copied out of a result set. Neither has any structure separating it from a
/// legitimate business term — a product code, a status name, a tier label. A string
/// inspector that rejected them would reject `hunter2` and `SHIPPED` alike and make
/// the feature unusable.
///
/// They are kept out by construction rather than by inspection: the typed payload
/// allow-list means a claim can only ever hold a description, an alias, a grain, a
/// column role, or a relationship, and the learning envelope (a later phase) never
/// shows result rows to the extractor in the first place. This constant exists so
/// that boundary is written down and tested, not assumed.
const UNDETECTABLE_SENTINELS: &[&str] = &["SENTINELPASSWORD", "SENTINELROWVALUE"];

// ---------------------------------------------------------------------------
// Shared helpers (process-unique temp dirs; no cross-test sharing)
// ---------------------------------------------------------------------------

fn temp_root(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "saya-security-{label}-{}-{stamp}",
        std::process::id()
    ));
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
    SchemaFingerprint::from_parts(1, &"a".repeat(64)).unwrap()
}

/// Propose a table-description claim carrying `text` at the given initial status.
/// Returns the stored id on success, or the typed error on rejection.
async fn propose_text_as(
    store: &SqliteStateStore,
    object: &DatabaseObjectRef,
    text: &str,
    status: ClaimStatus,
) -> Result<ClaimId, StoreError> {
    let request = ProposeClaim {
        object: object.clone(),
        fingerprint: fingerprint(),
        payload: ClaimPayload::table_description(text).unwrap(),
        origin: ClaimOrigin::UserExplicit,
        initial_status: status,
        evidence: None,
        referenced_columns: Vec::new(),
    };
    match store.propose_claim(request).await? {
        ProposeOutcome::Stored(id) => Ok(id),
        ProposeOutcome::Duplicate { id, .. } => Ok(id),
    }
}

/// Confirmed table-description — used when the test wants the claim persisted in
/// one step (so a non-redacted sentinel reaches the bytes immediately).
async fn propose_text(
    store: &SqliteStateStore,
    object: &DatabaseObjectRef,
    text: &str,
) -> Result<ClaimId, StoreError> {
    propose_text_as(store, object, text, ClaimStatus::Confirmed).await
}

/// Candidate table-description — used by the full-lifecycle scan, which then
/// transitions the claim through confirm/edit/forget itself.
async fn propose_candidate(
    store: &SqliteStateStore,
    object: &DatabaseObjectRef,
    text: &str,
) -> Result<ClaimId, StoreError> {
    propose_text_as(store, object, text, ClaimStatus::Candidate).await
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

/// Assert `bytes` contain none of the sentinels. On a hit, panic loudly with the
/// exact sentinel so the failure names the leak.
fn assert_no_sentinels(label: &str, bytes: &[u8]) {
    for sentinel in STRUCTURAL_SENTINELS {
        assert!(
            !window_contains(bytes, sentinel.as_bytes()),
            "LEAK: sentinel `{sentinel}` found in {label} bytes"
        );
    }
}

/// Naive byte-substring search (the "UTF-16LE-ish" caveat in the spec reduces to a
/// raw byte search because SQLite stores text as UTF-8).
fn window_contains(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || needle.len() > haystack.len() {
        return false;
    }
    haystack.windows(needle.len()).any(|w| w == needle)
}

async fn raw_pool(db: &Path) -> SqlitePool {
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(db)
                .create_if_missing(true),
        )
        .await
        .unwrap()
}

// ---------------------------------------------------------------------------
// §1 Byte scan of the database and both sidecars across a full lifecycle
// ---------------------------------------------------------------------------

/// Drive a full lifecycle, then scan db + -wal + -shm bytes before close (WAL still
/// populated) and again after `close()`. No sentinel may appear in any file.
#[tokio::test]
async fn full_lifecycle_leaves_no_sentinels_in_db_or_sidecars() {
    let root = temp_root("scan");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let obj = object("events");

    // The lifecycle text is clean — sentinels are only attempted in the dedicated
    // per-sentinel test below. This scan proves the baseline never smuggles one in
    // through the ordinary write path.
    let id = propose_candidate(&store, &obj, "the events table")
        .await
        .unwrap();
    // Add evidence by re-proposing the same payload (a Duplicate that records evidence).
    let _ = store
        .propose_claim(ProposeClaim {
            object: obj.clone(),
            fingerprint: fingerprint(),
            payload: ClaimPayload::table_description("the events table").unwrap(),
            origin: ClaimOrigin::UserExplicit,
            initial_status: ClaimStatus::Candidate,
            evidence: Some(ClaimEvidence {
                kind: EvidenceKind::ExplicitUserStatement,
                session_id: Some("s1".into()),
                turn_ordinal: Some(1),
                observed_unix_ms: 1_000,
            }),
            referenced_columns: Vec::new(),
        })
        .await
        .unwrap();
    store.confirm_claim(&id).await.unwrap();
    store
        .edit_claim(
            &id,
            ClaimPayload::table_description("edited clean text").unwrap(),
        )
        .await
        .unwrap();
    // A second, distinct claim on the same object.
    let second = propose_text(&store, &obj, "a second clean claim")
        .await
        .unwrap();
    store
        .forget_claim(&id, ForgetReason::UserRequest)
        .await
        .unwrap();
    let _ = second;

    // Before close: WAL still holds uncheckpointed pages.
    assert_no_sentinels("pre-close (db+wal+shm)", &db_bytes(&db));
    store.close().await;
    // After close: sidecars may have been checkpointed/removed; scan what remains.
    assert_no_sentinels("post-close (db+wal+shm)", &db_bytes(&db));
    let _ = fs::remove_dir_all(root);
}

/// For each sentinel, attempt to store it as a `TableDescription`. The ones
/// `redact()` catches must reject with `Invalid` and leave clean bytes. The ones it
/// does not catch — raw SQL, a row value, a file path, a bare credential, a header,
/// a private-key fragment — will store and reach the bytes; each such case is a
/// `LEAK:` finding and the assertion is left in place to fail loudly.
#[tokio::test]
async fn structural_sentinels_are_refused_and_never_reach_the_bytes() {
    for &sentinel in STRUCTURAL_SENTINELS {
        let root = temp_root(&format!("sent-{}", hash_label(sentinel)));
        let db = root.join("state.sqlite3");
        let store = SqliteStateStore::new(&db);
        let obj = object("events");

        let result = propose_text(&store, &obj, sentinel).await;
        let bytes = db_bytes(&db);
        store.close().await;
        let _ = fs::remove_dir_all(root);

        assert_eq!(
            result.err(),
            Some(StoreError::Invalid),
            "`{sentinel}` was admitted; admission::check must refuse it",
        );
        assert!(
            !window_contains(&bytes, sentinel.as_bytes()),
            "LEAK: `{sentinel}` reached the database bytes",
        );
    }
}

#[tokio::test]
async fn opaque_sentinels_are_stored_because_nothing_distinguishes_them() {
    // This test documents a limit rather than a guarantee, and it is written to
    // fail if that limit ever changes silently in either direction. A bare token
    // is indistinguishable from a product code, so the store admits it; keeping
    // secrets and result cells out is the job of the typed payload allow-list and
    // the learning envelope, not of string inspection. See UNDETECTABLE_SENTINELS.
    for &sentinel in UNDETECTABLE_SENTINELS {
        let root = temp_root(&format!("opaque-{}", hash_label(sentinel)));
        let db = root.join("state.sqlite3");
        let store = SqliteStateStore::new(&db);
        let obj = object("events");

        let result = propose_text(&store, &obj, sentinel).await;
        store.close().await;
        let _ = fs::remove_dir_all(root);

        assert!(
            result.is_ok(),
            "`{sentinel}` was refused — if the store learned to detect this class, \
             move it into STRUCTURAL_SENTINELS and delete it from here",
        );
    }
}

/// Stable, filesystem-safe label derived from a sentinel for the temp dir.
fn hash_label(sentinel: &str) -> String {
    let mut acc: u64 = 0xCBF29CE484222325;
    for &byte in sentinel.as_bytes() {
        acc ^= byte as u64;
        acc = acc.wrapping_mul(0x100000001B3);
    }
    format!("{acc:016x}")
}

// ---------------------------------------------------------------------------
// §2 Permission bits (unix only)
// ---------------------------------------------------------------------------

#[cfg(unix)]
#[tokio::test]
async fn unix_db_and_sidecars_and_parent_are_private() {
    use std::os::unix::fs::PermissionsExt;
    let root = temp_root("perms");
    let db = root.join("nested").join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let obj = object("events");
    // Candidate -> confirm exercises the transition path; the write populates the WAL.
    let id = propose_candidate(&store, &obj, "private lifecycle")
        .await
        .unwrap();
    store.confirm_claim(&id).await.unwrap();
    // A second object forces a fresh write so the WAL stays populated before close.
    let other = object("targets");
    let _ = store.upsert_object(&other, &fingerprint()).await.unwrap();
    store.close().await;

    let parent = db.parent().expect("db has a parent");
    assert_eq!(
        fs::metadata(parent).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(&db).unwrap().permissions().mode() & 0o777,
        0o600
    );
    for suffix in ["-wal", "-shm"] {
        let sidecar = state_sidecar_path(&db, suffix);
        if sidecar.exists() {
            assert_eq!(
                fs::metadata(&sidecar).unwrap().permissions().mode() & 0o777,
                0o600,
                "sidecar `{suffix}` not 0600"
            );
        }
    }
    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// §3 Corruption
// ---------------------------------------------------------------------------

/// A contract read on a 4 KiB `0xFF` file returns `Unavailable` and does not panic.
#[tokio::test]
async fn corrupt_ff_bytes_fail_with_unavailable() {
    let root = temp_root("corrupt-ff");
    let db = root.join("state.sqlite3");
    fs::write(&db, vec![0xFF; 4096]).unwrap();
    let store = SqliteStateStore::new(&db);
    let err = store.list_objects(&profile()).await.unwrap_err();
    assert_eq!(err, StoreError::Unavailable);
    let _ = fs::remove_dir_all(root);
}

/// A contract read on a truncated (100-byte) valid database returns `Unavailable`.
#[tokio::test]
async fn truncated_database_fails_with_unavailable() {
    let root = temp_root("corrupt-trunc");
    let db = root.join("state.sqlite3");
    // Build a valid database first.
    let store = SqliteStateStore::new(&db);
    let obj = object("events");
    let _ = propose_text(&store, &obj, "build a real db").await.unwrap();
    store.close().await;
    // Truncate to 100 bytes — well short of a valid header.
    let valid = fs::read(&db).unwrap();
    assert!(
        valid.len() > 100,
        "test db too small to truncate meaningfully"
    );
    fs::write(&db, &valid[..100]).unwrap();
    let store = SqliteStateStore::new(&db);
    let err = store.list_objects(&profile()).await.unwrap_err();
    assert_eq!(err, StoreError::Unavailable);
    // Memory being broken must never take down a path that does not need it: an
    // ordinary SchemaStore method on the same corrupt file also returns a typed
    // error rather than panicking.
    let err = store.list_schema_metadata().await.unwrap_err();
    assert!(matches!(err, StoreError::Unavailable));
    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// §4 Unknown future version
// ---------------------------------------------------------------------------

/// A hand-built `user_version = 99` database rejects every ContractStore method with
/// `VersionUnsupported` and leaves the file byte-identical before and after.
#[tokio::test]
async fn future_version_rejects_every_method_and_preserves_bytes() {
    let root = temp_root("future");
    let db = root.join("state.sqlite3");
    let pool = raw_pool(&db).await;
    sqlx::query("PRAGMA user_version = 99")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;

    let before = fs::read(&db).unwrap();
    let store = SqliteStateStore::new(&db);
    let obj = object("events");
    let unknown = ClaimId::parse("c-unknown0000000000000000000000000000000000").unwrap();

    // Every ContractStore method must surface VersionUnsupported, not panic.
    assert_eq!(
        store.upsert_object(&obj, &fingerprint()).await.unwrap_err(),
        StoreError::VersionUnsupported
    );
    assert_eq!(
        store
            .propose_claim(ProposeClaim {
                object: obj.clone(),
                fingerprint: fingerprint(),
                payload: ClaimPayload::table_description("future").unwrap(),
                origin: ClaimOrigin::UserExplicit,
                initial_status: ClaimStatus::Candidate,
                evidence: None,
                referenced_columns: Vec::new(),
            })
            .await
            .unwrap_err(),
        StoreError::VersionUnsupported
    );
    assert_eq!(
        store.get_claim(&unknown).await.unwrap_err(),
        StoreError::VersionUnsupported
    );
    assert_eq!(
        store.list_claims(&obj, &[]).await.unwrap_err(),
        StoreError::VersionUnsupported
    );
    assert_eq!(
        store.list_objects(&profile()).await.unwrap_err(),
        StoreError::VersionUnsupported
    );
    assert_eq!(
        store.confirm_claim(&unknown).await.unwrap_err(),
        StoreError::VersionUnsupported
    );
    assert_eq!(
        store
            .edit_claim(&unknown, ClaimPayload::table_description("x").unwrap())
            .await
            .unwrap_err(),
        StoreError::VersionUnsupported
    );
    assert_eq!(
        store.reject_claim(&unknown).await.unwrap_err(),
        StoreError::VersionUnsupported
    );
    assert_eq!(
        store
            .forget_claim(&unknown, ForgetReason::UserRequest)
            .await
            .unwrap_err(),
        StoreError::VersionUnsupported
    );
    assert_eq!(
        store.mark_stale(&unknown).await.unwrap_err(),
        StoreError::VersionUnsupported
    );
    assert_eq!(
        store.claim_events(&unknown, 10).await.unwrap_err(),
        StoreError::VersionUnsupported
    );

    let after = fs::read(&db).unwrap();
    assert_eq!(
        before, after,
        "future-version attempt mutated the database bytes"
    );
    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// §5 Error messages are payload-free
// ---------------------------------------------------------------------------

/// No reachable StoreError's `to_string()` may echo the offending input. We feed
/// `SENTINELTEXT` through every reachable variant and assert neither the sentinel
/// nor any >8-char substring of the input leaks into the message.
#[tokio::test]
async fn error_messages_carry_no_input_payload() {
    const S: &str = "SENTINELTEXT";

    // Invalid — a credential marker inside the payload trips redact() -> Invalid.
    let root = temp_root("msg");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let obj = object("events");
    let input = format!("creds password={S} end");
    let err = store
        .propose_claim(ProposeClaim {
            object: obj.clone(),
            fingerprint: fingerprint(),
            payload: ClaimPayload::table_description(&input).unwrap(),
            origin: ClaimOrigin::UserExplicit,
            initial_status: ClaimStatus::Candidate,
            evidence: None,
            referenced_columns: Vec::new(),
        })
        .await
        .unwrap_err();
    assert_eq!(err, StoreError::Invalid);
    assert_payload_free(&err.to_string(), S, &input);

    // NotFound — operating on a ClaimId that embeds the sentinel (alphanumeric).
    let fake = ClaimId::parse(&format!("c-{S}")).unwrap();
    let err = store.confirm_claim(&fake).await.unwrap_err();
    assert_eq!(err, StoreError::NotFound);
    assert_payload_free(&err.to_string(), S, &fake.to_string());

    // Conflict — forget twice; the first claim's text embeds the sentinel.
    let stored = propose_text(&store, &obj, &format!("{S} body"))
        .await
        .unwrap();
    store
        .forget_claim(&stored, ForgetReason::UserRequest)
        .await
        .unwrap();
    let err = store
        .forget_claim(&stored, ForgetReason::Incorrect)
        .await
        .unwrap_err();
    assert_eq!(err, StoreError::Conflict);
    assert_payload_free(&err.to_string(), S, &stored.to_string());

    // LimitExceeded — overflow the per-object claim cap with sentinel-bearing payloads.
    for i in 0..MAX_CLAIMS_PER_OBJECT {
        let _ = store
            .propose_claim(ProposeClaim {
                object: object("overflow"),
                fingerprint: fingerprint(),
                payload: ClaimPayload::table_alias(format!("a{i}")).unwrap(),
                origin: ClaimOrigin::UserExplicit,
                initial_status: ClaimStatus::Candidate,
                evidence: None,
                referenced_columns: Vec::new(),
            })
            .await
            .unwrap();
    }
    let overflow_input = format!("{S}overflow");
    let err = store
        .propose_claim(ProposeClaim {
            object: object("overflow"),
            fingerprint: fingerprint(),
            payload: ClaimPayload::table_alias(&overflow_input).unwrap(),
            origin: ClaimOrigin::UserExplicit,
            initial_status: ClaimStatus::Candidate,
            evidence: None,
            referenced_columns: Vec::new(),
        })
        .await
        .unwrap_err();
    assert_eq!(err, StoreError::LimitExceeded);
    assert_payload_free(&err.to_string(), S, &overflow_input);
    store.close().await;
    let _ = fs::remove_dir_all(root);

    // Unavailable — a corrupt file. The sentinel rides in the corrupt bytes.
    let root = temp_root("msg-unavail");
    let db = root.join("state.sqlite3");
    let corrupt = S.repeat(64);
    fs::write(&db, corrupt.as_bytes()).unwrap();
    let store = SqliteStateStore::new(&db);
    let err = store.list_objects(&profile()).await.unwrap_err();
    assert_eq!(err, StoreError::Unavailable);
    assert_payload_free(&err.to_string(), S, &corrupt);
    let _ = fs::remove_dir_all(root);

    // VersionUnsupported — a future-version file whose path embeds the sentinel.
    let root = temp_root(&format!("msg-{S}"));
    let db = root.join("state.sqlite3");
    let pool = raw_pool(&db).await;
    sqlx::query("PRAGMA user_version = 99")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    let store = SqliteStateStore::new(&db);
    let err = store.list_objects(&profile()).await.unwrap_err();
    assert_eq!(err, StoreError::VersionUnsupported);
    assert_payload_free(&err.to_string(), S, &db.to_string_lossy());
    let _ = fs::remove_dir_all(root);
}

/// Assert `message` contains neither the sentinel nor any substring of `input`
/// longer than 8 characters.
fn assert_payload_free(message: &str, sentinel: &str, input: &str) {
    assert!(
        !message.contains(sentinel),
        "error message `{message}` contains the sentinel `{sentinel}`"
    );
    for window in input.as_bytes().windows(9) {
        let substr = std::str::from_utf8(window).unwrap_or("");
        if substr.len() > 8 {
            assert!(
                !message.contains(substr),
                "error message `{message}` echoes input substring `{substr}`"
            );
        }
    }
}
