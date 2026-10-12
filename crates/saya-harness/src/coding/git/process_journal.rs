//! Private append-only persistence for bounded Git process receipts.
use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::Path,
    sync::{Arc, Mutex},
};

use super::process_observer::GitProcessReceipt;
use fs::{lock_file, read_records, unlock_file, validate_file};
use session::{append_locked, close_locked};

mod fs;
mod session;
#[path = "process_journal_wire.rs"]
pub(super) mod wire;
pub(super) use wire::encode_receipt;

#[cfg(test)]
#[path = "process_journal_tests.rs"]
mod tests;

const JOURNAL_NAME: &str = "git-processes.jsonl";
const MAX_BYTES: usize = 16 * 1024;
const MAX_RECEIPTS: usize = 256;
const MAX_RECORDS: usize = 512;

#[derive(Debug)]
pub(super) enum JournalAppendError {
    AlreadyIncomplete,
    Bounded,
    Io,
}

pub(super) struct JournalState {
    pub(super) receipts: Vec<GitProcessReceipt>,
    pub(super) incomplete: bool,
    pub(super) retained_bytes: usize,
    pub(super) session_started: bool,
}

pub(super) struct GitProcessJournal {
    file: Mutex<File>,
    appendable: Mutex<bool>,
    retained_records: Mutex<usize>,
    retained_bytes: Mutex<usize>,
    active_identity: Mutex<Option<String>>,
}

impl GitProcessJournal {
    pub(super) fn open(
        root: &Path,
        service_identity: &str,
    ) -> std::io::Result<(Arc<Self>, JournalState)> {
        let path = root.join(JOURNAL_NAME);
        let mut options = OpenOptions::new();
        options.read(true).write(true).append(true).create(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        let mut file = options.open(path)?;
        validate_file(&file)?;
        lock_file(&file)?;
        let (receipts, mut incomplete, mut bytes, mut records, open_sessions) =
            read_records(&file)?;
        let mut session_started = false;
        let mut appendable = !incomplete && open_sessions.is_empty();
        if appendable {
            let line = wire::encode_session_start(service_identity)
                .map_err(|_| std::io::Error::other("invalid session identity"))?;
            if records >= MAX_RECORDS || bytes.saturating_add(line.len()) > MAX_BYTES {
                appendable = false;
                incomplete = true;
            } else {
                file.write_all(&line)?;
                file.sync_all()?;
                records += 1;
                bytes += line.len();
                session_started = true;
            }
        } else if !open_sessions.is_empty() {
            incomplete = true;
        }
        unlock_file(&file);
        Ok((
            Arc::new(Self {
                file: Mutex::new(file),
                appendable: Mutex::new(appendable),
                retained_records: Mutex::new(records),
                retained_bytes: Mutex::new(bytes),
                active_identity: Mutex::new(session_started.then(|| service_identity.to_owned())),
            }),
            JournalState {
                receipts,
                incomplete,
                retained_bytes: bytes,
                session_started,
            },
        ))
    }

    pub(super) fn append(&self, receipt: &GitProcessReceipt) -> Result<(), JournalAppendError> {
        let active_identity = self.lock(&self.active_identity).clone();
        let mut appendable = self.lock(&self.appendable);
        if !*appendable {
            return Err(JournalAppendError::AlreadyIncomplete);
        }
        if active_identity.as_deref() != Some(receipt.service_identity.as_str()) {
            *appendable = false;
            return Err(JournalAppendError::AlreadyIncomplete);
        }
        let line = encode_receipt(receipt).map_err(|_| JournalAppendError::Io)?;
        let mut file = self.lock(&self.file);
        lock_file(&file).map_err(|_| JournalAppendError::Io)?;
        let result = append_locked(
            self,
            &mut file,
            &line,
            active_identity.as_deref(),
            &mut appendable,
        );
        unlock_file(&file);
        result
    }

    pub(super) fn close_session(&self) -> Result<(), JournalAppendError> {
        let identity = self
            .lock(&self.active_identity)
            .clone()
            .ok_or(JournalAppendError::AlreadyIncomplete)?;
        let mut appendable = self.lock(&self.appendable);
        if !*appendable {
            return Err(JournalAppendError::AlreadyIncomplete);
        }
        let line = wire::encode_session_close(&identity).map_err(|_| JournalAppendError::Io)?;
        let mut file = self.lock(&self.file);
        lock_file(&file).map_err(|_| JournalAppendError::Io)?;
        let result = close_locked(self, &mut file, &line, &identity, &mut appendable);
        unlock_file(&file);
        if result.is_ok() {
            drop(appendable);
            *self.lock(&self.active_identity) = None;
        }
        result
    }

    fn lock<'a, T>(&self, mutex: &'a Mutex<T>) -> std::sync::MutexGuard<'a, T> {
        mutex
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn write_incomplete_marker(
    file: &mut File,
    length: usize,
    appendable: &mut bool,
) -> Result<(), JournalAppendError> {
    *appendable = false;
    if length.saturating_add(wire::INCOMPLETE_LINE.len()) <= MAX_BYTES {
        file.write_all(wire::INCOMPLETE_LINE)
            .and_then(|()| file.sync_all())
            .map_err(|_| JournalAppendError::Io)?;
    }
    Ok(())
}
