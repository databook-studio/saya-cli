//! The byte inspection of admission screening: known-secret content and
//! database-export magic. Runs after the path rules because it reads the
//! whole payload.

use crate::redact;

const PEM_BEGIN: &[u8] = b"-----BEGIN";
const PEM_KEY: &[u8] = b"PRIVATE KEY";
/// How far past a `-----BEGIN` marker a `PRIVATE KEY` label may sit and
/// still count — the truncated-block case included.
const PEM_KEY_WINDOW: usize = 128;

const PARQUET_MAGIC: &[u8] = b"PAR1";
const SQLITE_MAGIC: &[u8] = b"SQLite format 3\0";
const DUCKDB_MAGIC: &[u8] = b"DUCK";
const DUCKDB_MAGIC_OFFSET: usize = 8;

/// True when the bytes carry a known-secret shape: valid UTF-8 the shared
/// redactor would change, or non-UTF-8 bytes holding a private-key block
/// header (case-insensitive ASCII search over the whole slice).
pub(super) fn contains_secret(bytes: &[u8]) -> bool {
    match std::str::from_utf8(bytes) {
        Ok(text) => redact(text) != text,
        Err(_) => contains_pem_key_header(bytes),
    }
}

/// True when the bytes open like a database-export artifact: Parquet
/// (`PAR1`), SQLite (`SQLite format 3\0`), or DuckDB (`DUCK` at offset 8).
pub(super) fn is_database_export(bytes: &[u8]) -> bool {
    bytes.starts_with(PARQUET_MAGIC)
        || bytes.starts_with(SQLITE_MAGIC)
        || (bytes.len() >= DUCKDB_MAGIC_OFFSET + DUCKDB_MAGIC.len()
            && &bytes[DUCKDB_MAGIC_OFFSET..DUCKDB_MAGIC_OFFSET + DUCKDB_MAGIC.len()]
                == DUCKDB_MAGIC)
}

/// Case-insensitive scan for a `-----BEGIN` marker followed, within the
/// window, by a `PRIVATE KEY` label.
fn contains_pem_key_header(bytes: &[u8]) -> bool {
    let mut cursor = 0;
    while let Some(offset) = find_ignore_ascii_case(&bytes[cursor..], PEM_BEGIN) {
        let begin = cursor + offset;
        let end = (begin + PEM_BEGIN.len() + PEM_KEY_WINDOW).min(bytes.len());
        if find_ignore_ascii_case(&bytes[begin..end], PEM_KEY).is_some() {
            return true;
        }
        cursor = begin + PEM_BEGIN.len();
    }
    false
}

fn find_ignore_ascii_case(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    (0..=haystack.len() - needle.len())
        .find(|&start| haystack[start..start + needle.len()].eq_ignore_ascii_case(needle))
}
