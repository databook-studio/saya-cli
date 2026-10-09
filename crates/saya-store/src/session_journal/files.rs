//! File plumbing of the journal: torn-tail repair, bounded reads, line
//! parsing, 0600 append-mode opening, and the io error mapping.

use super::MAX_JOURNAL_BYTES;
use super::events::JournalEvent;
use crate::StoreError;
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::Path,
};

/// Drops a torn trailing line — the half-written event a crash left — so a
/// write cannot concatenate onto it. A whole-line journal is untouched, and
/// an absent journal creates nothing.
pub(super) fn truncate_torn_tail(path: &Path) {
    let Ok(metadata) = fs::metadata(path) else {
        return;
    };
    if metadata.len() > MAX_JOURNAL_BYTES as u64 {
        return;
    }
    let Ok(raw) = fs::read_to_string(path) else {
        return;
    };
    if raw.is_empty() || raw.ends_with('\n') {
        return;
    }
    let keep = raw.rfind('\n').map_or(0, |index| index + 1);
    if let Ok(mut file) = OpenOptions::new().write(true).truncate(true).open(path) {
        let _ = file.write_all(&raw.as_bytes()[..keep]);
        let _ = file.sync_all();
    }
}

pub(super) fn bounded_read(path: &Path) -> Result<Option<Vec<u8>>, StoreError> {
    let file = match fs::File::open(path) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(io_error(path, error)),
    };
    let mut bytes = Vec::new();
    file.take(MAX_JOURNAL_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| io_error(path, error))?;
    if bytes.len() > MAX_JOURNAL_BYTES {
        return Err(StoreError::LimitExceeded);
    }
    Ok(Some(bytes))
}

/// Parses every newline-terminated line. A torn final line — no newline —
/// was never a complete event and is ignored; a complete line that does not
/// parse fails closed.
pub(super) fn parse_lines(raw: &str) -> Result<Vec<JournalEvent>, StoreError> {
    let torn_tail = !raw.is_empty() && !raw.ends_with('\n');
    let total = raw.lines().count();
    raw.lines()
        .enumerate()
        .filter(|(index, line)| !line.is_empty() && !(torn_tail && index + 1 == total))
        .map(|(_, line)| serde_json::from_str(line).map_err(|_| StoreError::Invalid))
        .collect()
}

/// Opens the journal for appending, creating it 0600 — a session state dir
/// holds no readable-by-others bytes.
pub(super) fn open_append(path: &Path) -> Result<fs::File, StoreError> {
    // `mode` is an extension trait, so the import is cfg-gated with its use —
    // importing it unconditionally does not compile on Windows.
    #[cfg(unix)]
    use std::os::unix::fs::OpenOptionsExt;

    let mut options = OpenOptions::new();
    options.append(true).create(true);
    #[cfg(unix)]
    options.mode(0o600);
    options.open(path).map_err(|error| io_error(path, error))
}

/// `StoreError` is payload-free by contract, so an `io::Error` can only map
/// to the store's own unavailable shape.
pub(super) fn io_error(_: &Path, _: std::io::Error) -> StoreError {
    StoreError::Unavailable
}
