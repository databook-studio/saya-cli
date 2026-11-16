//! Journal file safety: metadata validation, advisory locking, and record re-scanning.

use std::{
    collections::HashSet,
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom},
    path::Path,
};

use super::wire;
use super::wire::decode_line;
use super::{GitProcessReceipt, MAX_BYTES, MAX_RECEIPTS, MAX_RECORDS};

/// Opens the journal for durable appends. Windows append handles
/// (`FILE_APPEND_DATA`) cannot be flushed (`FlushFileBuffers` needs
/// `FILE_WRITE_DATA`), so there the journal asks for ordinary write access
/// and every writer seeks to the end first; unix keeps its atomic append flag.
pub(super) fn open_append(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    options.write(true).create(true);
    #[cfg(not(windows))]
    options.append(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt as _;
        options.custom_flags(crate::workspace::contain::FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let mut file = options.open(path)?;
    seek_end(&mut file)?;
    Ok(file)
}

/// Positions the handle at the end before a write. A no-op on unix, where
/// append mode ignores the position.
pub(super) fn seek_end(file: &mut File) -> std::io::Result<()> {
    #[cfg(windows)]
    file.seek(SeekFrom::End(0))?;
    #[cfg(not(windows))]
    let _ = file;
    Ok(())
}

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
    #[cfg(windows)]
    if !metadata.is_file() || crate::workspace::windows_identity::is_reparse_point(file)? {
        return Err(std::io::Error::other("unsafe journal"));
    }
    #[cfg(all(not(unix), not(windows)))]
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
    #[cfg(windows)]
    file.lock()?;
    #[cfg(all(not(unix), not(windows)))]
    let _ = file;
    Ok(())
}
pub(super) fn unlock_file(file: &File) {
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        let _ = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_UN) };
    }
    #[cfg(windows)]
    let _ = file.unlock();
    #[cfg(all(not(unix), not(windows)))]
    let _ = file;
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

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;
    use std::io::Write as _;

    /// Real Windows refuses `FlushFileBuffers` on an append-only handle
    /// (`FILE_APPEND_DATA` without `FILE_WRITE_DATA`). An append handle also
    /// ignores the write position, so a write at offset zero proves the
    /// returned handle carries ordinary write access.
    #[test]
    fn append_handle_flushes_and_writes_at_the_seek_position() {
        let path = std::env::temp_dir().join(format!(
            "saya-process-journal-append-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut file = open_append(&path).unwrap();
        file.write_all(b"first\n").unwrap();
        file.sync_all().unwrap();
        seek_end(&mut file).unwrap();
        file.write_all(b"second\n").unwrap();
        file.sync_all().unwrap();
        drop(file);
        assert_eq!(std::fs::read(&path).unwrap(), b"first\nsecond\n");
        let mut file = open_append(&path).unwrap();
        file.seek(SeekFrom::Start(0)).unwrap();
        file.write_all(b"X").unwrap();
        file.sync_all().unwrap();
        drop(file);
        assert_eq!(std::fs::read(&path).unwrap(), b"Xirst\nsecond\n");
        let _ = std::fs::remove_file(&path);
    }
}
