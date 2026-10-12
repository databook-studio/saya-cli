//! Journal file safety: metadata validation, advisory locking, and record re-scanning.

use std::{
    collections::HashSet,
    fs::File,
    io::{Read, Seek, SeekFrom},
};

use super::wire;
use super::wire::decode_line;
use super::{GitProcessReceipt, MAX_BYTES, MAX_RECEIPTS, MAX_RECORDS};

pub(super) fn validate_file(file: &File) -> std::io::Result<()> {
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

pub(super) fn lock_file(file: &File) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}
pub(super) fn unlock_file(file: &File) {
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        let _ = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_UN) };
    }
}

/// What one journal read settles: the receipts in order, whether the tail
/// was truncated or corrupt, the valid-byte and record accounting, and the
/// sessions still open.
type JournalRead = (Vec<GitProcessReceipt>, bool, usize, usize, HashSet<String>);

pub(super) fn read_records(file: &File) -> std::io::Result<JournalRead> {
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
