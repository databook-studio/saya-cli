//! Reading and validating staged-source snapshots: the pinned DuckDB
//! configuration every open of a staged file uses (external access off,
//! autoload off, configuration locked), the metadata read-back, and the
//! "is this directory a saya-staged snapshot" predicate shared by reuse,
//! replacement, and cleanup. A snapshot's directory name carries the
//! content-hash prefix and a short digest of its stored parse contract
//! (`<sha16>-<digest8>`); legacy snapshots under the bare 16-hex content
//! prefix remain valid — listed and cleaned, but never reused.

use std::{collections::HashMap, fs, path::Path};

use duckdb::{AccessMode, Config, Connection};
use saya_harness::file_source::{RESERVED_METADATA_TABLE, STAGED_DB_FILE};

use super::contract;

/// What a staged snapshot says about itself, read back from its metadata.
/// `contract`/`preview` are absent on legacy snapshots staged before
/// contracts existed; such snapshots are never reused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SnapshotMeta {
    pub file_name: String,
    pub sha256: String,
    pub bytes: u64,
    pub rows: u64,
    pub columns: u64,
    pub staged_unix_ms: u64,
    pub format: String,
    pub contract: Option<String>,
    pub preview: Option<String>,
}

/// The staging configuration, pinned the same way `saya-harness` pins the
/// staged and scratch files: external access off, autoload and community
/// extensions off, no persistent secrets, configuration locked.
pub(super) fn staged_config(read_only: bool) -> Result<Config, String> {
    Config::default()
        .access_mode(if read_only {
            AccessMode::ReadOnly
        } else {
            AccessMode::ReadWrite
        })
        .and_then(|item| item.enable_external_access(false))
        .and_then(|item| item.enable_autoload_extension(false))
        .and_then(|item| item.with("allow_community_extensions", "false"))
        .and_then(|item| item.with("allow_persistent_secrets", "false"))
        .and_then(|item| item.with("lock_configuration", "true"))
        .map_err(|_| "duckdb snapshot configuration is invalid".to_owned())
}

/// Whether `sha256` is a 64-character lowercase-hex content digest.
fn is_content_sha(sha256: &str) -> bool {
    sha256.len() == 64
        && sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

/// The snapshot directory name for content `sha256` under `contract`: the
/// 16-hex content prefix, a dash, and the contract's 8-hex digest.
pub(super) fn dir_name(sha256: &str, contract: &str) -> Option<String> {
    is_content_sha(sha256).then(|| format!("{}-{}", &sha256[..16], contract::digest(contract)))
}

fn is_sha16(name: &str) -> bool {
    name.len() == 16
        && name
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_digest8(name: &str) -> bool {
    name.len() == 8
        && name
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Whether the name is the contract-suffixed layout `<sha16>-<digest8>`.
fn is_contract_name(name: &str) -> bool {
    name.split_once('-')
        .is_some_and(|(sha16, digest)| is_sha16(sha16) && is_digest8(digest))
}

/// Reads the metadata table from a staged snapshot file. Any failure —
/// unreadable file, missing table, absent or malformed keys — is `None`: the
/// caller treats the snapshot as not saya's.
pub(super) fn snapshot_meta(db_path: &Path) -> Option<SnapshotMeta> {
    let file = fs::symlink_metadata(db_path).ok()?;
    if file.is_symlink() || !file.is_file() {
        return None;
    }
    let connection = Connection::open_with_flags(db_path, staged_config(true).ok()?).ok()?;
    let mut statement = connection
        .prepare(&format!("SELECT key, value FROM {RESERVED_METADATA_TABLE}"))
        .ok()?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .ok()?;
    let mut values = HashMap::<String, String>::new();
    for row in rows {
        let (key, value) = row.ok()?;
        values.insert(key, value);
    }
    let text = |key: &str| values.get(key).filter(|value| !value.is_empty()).cloned();
    let number = |key: &str| -> Option<u64> { values.get(key)?.parse().ok() };
    Some(SnapshotMeta {
        file_name: text("file_name")?,
        sha256: text("sha256")?,
        bytes: number("bytes")?,
        rows: number("rows")?,
        columns: number("columns")?,
        staged_unix_ms: number("staged_unix_ms")?,
        format: text("format")?,
        contract: text("contract"),
        preview: text("preview"),
    })
    // Both staged formats (C1 CSV, C2 Parquet) carry the same metadata keys.
    .filter(|meta| {
        is_content_sha(&meta.sha256) && matches!(meta.format.as_str(), "csv" | "parquet")
    })
}

/// Whether `dir` is a saya-staged snapshot: a real directory (not a symlink)
/// whose name — the contract-suffixed `<sha16>-<digest8>` or the legacy bare
/// 16-hex content prefix — matches the sha256 its valid metadata carries.
/// A contract-suffixed name must also re-derive its digest from the stored
/// contract, so name and stored contract cannot disagree.
pub(super) fn valid_snapshot(dir: &Path) -> Option<SnapshotMeta> {
    let meta = fs::symlink_metadata(dir).ok()?;
    if meta.is_symlink() || !meta.is_dir() {
        return None;
    }
    let name = dir.file_name()?.to_str()?;
    if !(is_sha16(name) || is_contract_name(name)) {
        return None;
    }
    let snapshot = snapshot_meta(&dir.join(STAGED_DB_FILE))?;
    if is_sha16(name) {
        return (snapshot.sha256.get(..16) == Some(name)).then_some(snapshot);
    }
    let (sha16, digest) = name.split_once('-')?;
    let contract = snapshot.contract.as_deref()?;
    (snapshot.sha256.get(..16) == Some(sha16) && contract::digest(contract) == digest)
        .then_some(snapshot)
}
