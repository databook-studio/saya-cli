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
    CleanupState, ForgetOutcome, KnowledgeItemRequest, KnowledgeItemStore, KnowledgeStoreError,
    SqliteStateStore, StoreError, state_sidecar_path,
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

/// A sentinel planted in a fact's free-text value to prove `forget` erases it
/// from the bytes, not merely from the API's view. Distinct from the structural
/// sentinels above: it is ordinary business text that the admission gate
/// legitimately admits (it is not a credential shape), so it reaches the
/// database bytes on the write — and `forget` must remove it from there.
const FORGET_SENTINEL: &str = "SENTINELFORGETCONTENT";

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

#[path = "knowledge_security/binding_admission.rs"]
mod binding_admission;
#[path = "knowledge_security/forget_value.rs"]
mod forget_value;
#[path = "knowledge_security/lifecycle.rs"]
mod lifecycle;
#[path = "knowledge_security/opaque_limit.rs"]
mod opaque_limit;
#[path = "knowledge_security/reason_security.rs"]
mod reason_security;
#[path = "knowledge_security/value_admission.rs"]
mod value_admission;
