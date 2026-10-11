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

#[derive(Debug)]
pub(super) enum JournalAppendError {
    AlreadyIncomplete,
    Bounded,
    Io,
}

pub(super) struct JournalState {
    pub(super) receipts: Vec<GitProcessReceipt>,
    pub(super) incomplete: bool,
}

pub(super) struct GitProcessJournal {
    file: Mutex<File>,
    appendable: Mutex<bool>,
    retained_records: Mutex<usize>,
    retained_bytes: Mutex<usize>,
}

impl GitProcessJournal {
    pub(super) fn open(root: &Path) -> std::io::Result<(Arc<Self>, JournalState)> {
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
        let file = options.open(path)?;
        validate_file(&file)?;
        let (receipts, incomplete, bytes) = read_records(&file)?;
        Ok((
            Arc::new(Self {
                file: Mutex::new(file),
                appendable: Mutex::new(!incomplete),
                retained_records: Mutex::new(receipts.len()),
                retained_bytes: Mutex::new(bytes),
            }),
            JournalState {
                receipts,
                incomplete,
            },
        ))
    }

    pub(super) fn append(&self, receipt: &GitProcessReceipt) -> Result<(), JournalAppendError> {
        let mut appendable = self.lock(&self.appendable);
        if !*appendable {
            return Err(JournalAppendError::AlreadyIncomplete);
        }
        let line = encode_receipt(receipt).map_err(|_| JournalAppendError::Io)?;
        let mut file = self.lock(&self.file);
        lock_file(&file).map_err(|_| JournalAppendError::Io)?;
        let result = append_locked(self, &mut file, &line, &mut appendable);
        unlock_file(&file);
        result
    }

    fn lock<T>(&self, mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
        mutex
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn append_locked(
    journal: &GitProcessJournal,
    file: &mut File,
    line: &[u8],
    appendable: &mut bool,
) -> Result<(), JournalAppendError> {
    validate_file(file).map_err(|_| JournalAppendError::Io)?;
    let (persisted, incomplete, persisted_bytes) =
        read_records(file).map_err(|_| JournalAppendError::Io)?;
    if incomplete {
        *appendable = false;
        return Err(JournalAppendError::AlreadyIncomplete);
    }
    *journal.lock(&journal.retained_records) = persisted.len();
    *journal.lock(&journal.retained_bytes) = persisted_bytes;
    let length = file.metadata().map_err(|_| JournalAppendError::Io)?.len() as usize;
    let count = persisted.len();
    let retained = persisted_bytes;
    if line.len() > wire::MAX_LINE
        || count >= MAX_RECEIPTS
        || retained.saturating_add(line.len()) > MAX_BYTES
    {
        if length.saturating_add(wire::INCOMPLETE_LINE.len()) <= MAX_BYTES {
            file.write_all(wire::INCOMPLETE_LINE)
                .and_then(|()| file.sync_all())
                .map_err(|_| JournalAppendError::Io)?;
        }
        *appendable = false;
        return Err(JournalAppendError::Bounded);
    }
    file.write_all(line)
        .and_then(|()| file.sync_all())
        .map_err(|_| {
            *appendable = false;
            JournalAppendError::Io
        })?;
    *journal.lock(&journal.retained_records) += 1;
    *journal.lock(&journal.retained_bytes) += line.len();
    Ok(())
}

fn read_records(file: &File) -> std::io::Result<(Vec<GitProcessReceipt>, bool, usize)> {
    let mut reader = file.try_clone()?;
    reader.seek(SeekFrom::Start(0))?;
    let mut bytes = Vec::new();
    reader
        .take((MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_BYTES {
        return Ok((Vec::new(), true, MAX_BYTES));
    }
    let mut receipts = Vec::new();
    let mut identities = HashSet::new();
    let mut valid_bytes = 0;
    let mut incomplete = false;
    for chunk in bytes.split_inclusive(|byte| *byte == b'\n') {
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
            Some(Ok(receipt)) => {
                if receipts.len() == MAX_RECEIPTS
                    || !identities.insert((receipt.service_identity.clone(), receipt.sequence))
                {
                    incomplete = true;
                    break;
                }
                receipts.push(receipt.into());
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
    Ok((receipts, incomplete, valid_bytes))
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
