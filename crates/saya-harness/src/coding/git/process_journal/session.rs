//! Bounded append and session-close accounting for an open journal file.

use std::{fs::File, io::Write};

use super::fs::{read_records, validate_file};
use super::wire;
use super::write_incomplete_marker;
use super::{GitProcessJournal, JournalAppendError, MAX_BYTES, MAX_RECEIPTS, MAX_RECORDS};

pub(super) fn append_locked(
    journal: &GitProcessJournal,
    file: &mut File,
    line: &[u8],
    active_identity: Option<&str>,
    appendable: &mut bool,
) -> Result<(), JournalAppendError> {
    validate_file(file).map_err(|_| JournalAppendError::Io)?;
    let (persisted, incomplete, persisted_bytes, records, open_sessions) =
        read_records(file).map_err(|_| JournalAppendError::Io)?;
    if incomplete || active_identity.is_none_or(|identity| !open_sessions.contains(identity)) {
        *appendable = false;
        return Err(JournalAppendError::AlreadyIncomplete);
    }
    *journal.lock(&journal.retained_records) = records;
    *journal.lock(&journal.retained_bytes) = persisted_bytes;
    let length = file.metadata().map_err(|_| JournalAppendError::Io)?.len() as usize;
    let count = persisted.len();
    let retained = persisted_bytes;
    if line.len() > wire::MAX_LINE
        || count >= MAX_RECEIPTS
        || records >= MAX_RECORDS
        || retained.saturating_add(line.len()) > MAX_BYTES
    {
        write_incomplete_marker(file, length, appendable)?;
        return Err(JournalAppendError::Bounded);
    }
    file.write_all(line)
        .and_then(|()| file.sync_all())
        .map_err(|_| {
            *appendable = false;
            JournalAppendError::Io
        })?;
    *journal.lock(&journal.retained_records) = records + 1;
    *journal.lock(&journal.retained_bytes) += line.len();
    Ok(())
}

pub(super) fn close_locked(
    journal: &GitProcessJournal,
    file: &mut File,
    line: &[u8],
    identity: &str,
    appendable: &mut bool,
) -> Result<(), JournalAppendError> {
    validate_file(file).map_err(|_| JournalAppendError::Io)?;
    let (_, incomplete, retained_bytes, records, open_sessions) =
        read_records(file).map_err(|_| JournalAppendError::Io)?;
    if incomplete || open_sessions.len() != 1 || !open_sessions.contains(identity) {
        *appendable = false;
        return Err(JournalAppendError::AlreadyIncomplete);
    }
    let length = file.metadata().map_err(|_| JournalAppendError::Io)?.len() as usize;
    if line.len() > wire::MAX_LINE
        || records >= MAX_RECORDS
        || retained_bytes.saturating_add(line.len()) > MAX_BYTES
    {
        write_incomplete_marker(file, length, appendable)?;
        return Err(JournalAppendError::Bounded);
    }
    file.write_all(line)
        .and_then(|()| file.sync_all())
        .map_err(|_| {
            *appendable = false;
            JournalAppendError::Io
        })?;
    *journal.lock(&journal.retained_records) = records + 1;
    *journal.lock(&journal.retained_bytes) = retained_bytes + line.len();
    Ok(())
}
