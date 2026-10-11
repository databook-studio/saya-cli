//! Private append-only persistence for bounded Git process receipts.
use std::{
    collections::HashSet,
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
    sync::{Arc, Mutex},
};

use super::process_observer::GitProcessReceipt;

#[path = "process_journal_wire.rs"]
pub(super) mod wire;
pub(super) use wire::{decode_line, encode_receipt};

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

fn append_locked(
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

fn close_locked(
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

fn read_records(
    file: &File,
) -> std::io::Result<(Vec<GitProcessReceipt>, bool, usize, usize, HashSet<String>)> {
    let mut reader = file.try_clone()?;
    reader.seek(SeekFrom::Start(0))?;
    let mut bytes = Vec::new();
    reader
        .take((MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_BYTES {
        return Ok((Vec::new(), true, MAX_BYTES, MAX_RECORDS, HashSet::new()));
    }
    let mut receipts = Vec::new();
    let mut identities = HashSet::new();
    let mut closed_sessions = HashSet::new();
    let mut open_sessions = HashSet::new();
    let mut records = 0;
    let mut valid_bytes: usize = 0;
    let mut incomplete = false;
    for chunk in bytes.split_inclusive(|byte| *byte == b'\n') {
        if records >= MAX_RECORDS {
            incomplete = true;
            break;
        }
        if !chunk.ends_with(b"\n") || chunk.len() > wire::MAX_LINE {
            incomplete = true;
            break;
        }
        let line = &chunk[..chunk.len() - 1];
        match decode_line(line) {
            Some(Err(())) => {
                incomplete = true;
                break;
            }
            Some(Ok(record)) => {
                let valid = match record {
                    wire::JournalLine::SessionStart(identity) => {
                        !open_sessions.contains(&identity) && {
                            closed_sessions.remove(&identity);
                            open_sessions.insert(identity)
                        }
                    }
                    wire::JournalLine::SessionClose(identity) => {
                        open_sessions.remove(&identity) && closed_sessions.insert(identity)
                    }
                    wire::JournalLine::Receipt(receipt) => {
                        receipts.len() < MAX_RECEIPTS
                            && open_sessions.contains(&receipt.service_identity)
                            && !closed_sessions.contains(&receipt.service_identity)
                            && identities
                                .insert((receipt.service_identity.clone(), receipt.sequence))
                            && {
                                receipts.push(receipt.into());
                                true
                            }
                    }
                };
                if !valid {
                    incomplete = true;
                    break;
                }
                records += 1;
                valid_bytes = valid_bytes.saturating_add(chunk.len());
            }
            None => {
                incomplete = true;
                break;
            }
        }
    }
    if !bytes.is_empty() && valid_bytes != bytes.len() && !incomplete {
        incomplete = true;
    }
    Ok((receipts, incomplete, valid_bytes, records, open_sessions))
}

fn validate_file(file: &File) -> std::io::Result<()> {
    let metadata = file.metadata()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if !metadata.is_file()
            || metadata.mode() & 0o077 != 0
            || metadata.uid() != unsafe { libc::geteuid() }
        {
            return Err(std::io::Error::other("unsafe journal"));
        }
    }
    #[cfg(not(unix))]
    if !metadata.is_file() {
        return Err(std::io::Error::other("unsafe journal"));
    }
    Ok(())
}

fn lock_file(file: &File) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}
fn unlock_file(file: &File) {
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        let _ = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_UN) };
    }
}

#[cfg(test)]
#[path = "process_journal_tests.rs"]
mod tests;
