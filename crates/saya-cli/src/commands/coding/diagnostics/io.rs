//! Fixed private paths and a stable nonrotating cross-process lock.
use super::{Fault, History, LOCK, STAGE, SUMMARY, SUMMARY_BYTES};
use std::{
    fs::{File, Metadata, OpenOptions},
    io::{Read, Write},
    path::Path,
    sync::{Mutex, MutexGuard},
};

// Filesystem locks arbitrate processes. Local readers and writers serialize so
// healthy parallel callbacks cannot turn their own overlap into an owner fault.
static PROCESS_LOCK: Mutex<()> = Mutex::new(());
pub(super) struct Lock {
    // Explicit unlock releases cloned or inherited descriptors before
    // PROCESS_LOCK permits the next local diagnostic operation.
    _file: File,
    _process: MutexGuard<'static, ()>,
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = File::unlock(&self._file);
    }
}

fn validate(metadata: &Metadata, directory: bool) -> Result<(), Fault> {
    if directory && !metadata.is_dir() || !directory && !metadata.is_file() {
        return Err(Fault::UnsafePath);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // Private owner-only paths also prevent another uid replacing our lock.
        if metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o077 != 0
            || (!directory && metadata.nlink() != 1)
        {
            return Err(Fault::UnsafePath);
        }
    }
    Ok(())
}
pub(super) fn open(root: &Path, name: &str, append: bool) -> Result<File, Fault> {
    let mut options = OpenOptions::new();
    options.read(true);
    if append {
        options.append(true).create(true);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let path = root.join(name);
    if let Ok(metadata) = std::fs::symlink_metadata(&path) {
        validate(&metadata, false)?;
    }
    let file = options.open(&path).map_err(|_| Fault::Storage)?;
    validate(&file.metadata().map_err(|_| Fault::Storage)?, false)?;
    Ok(file)
}
pub(super) fn lock(root: &Path) -> Result<Lock, Fault> {
    let process = PROCESS_LOCK.lock().map_err(|_| Fault::Storage)?;
    validate(
        &std::fs::symlink_metadata(root).map_err(|_| Fault::UnsafePath)?,
        true,
    )?;
    let file = open(root, LOCK, true)?;
    if file.metadata().map_err(|_| Fault::Storage)?.len() != 0 {
        return Err(Fault::UnsafePath);
    }
    match file.try_lock() {
        Ok(()) => Ok(Lock {
            _file: file,
            _process: process,
        }),
        Err(std::fs::TryLockError::WouldBlock) => Err(Fault::LockContended),
        Err(_) => Err(Fault::Storage),
    }
}
pub(super) fn length(root: &Path, name: &str, bound: usize) -> Result<Option<u64>, Fault> {
    match std::fs::symlink_metadata(root.join(name)) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(Fault::Storage),
        Ok(metadata) => {
            validate(&metadata, false)?;
            if metadata.len() > bound as u64 {
                return Err(Fault::Recovery);
            }
            Ok(Some(metadata.len()))
        }
    }
}
pub(super) fn read(root: &Path, name: &str, bound: usize) -> Result<Option<Vec<u8>>, Fault> {
    if length(root, name, bound)?.is_none() {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    open(root, name, false)?
        .take(bound as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Fault::Storage)?;
    if bytes.len() > bound {
        return Err(Fault::Recovery);
    }
    Ok(Some(bytes))
}
fn sync_dir(root: &Path) -> Result<(), Fault> {
    File::open(root)
        .and_then(|file| file.sync_all())
        .map_err(|_| Fault::Storage)
}
fn rewrite(root: &Path, name: &str, bytes: &[u8]) -> Result<(), Fault> {
    if bytes.len() > SUMMARY_BYTES {
        return Err(Fault::Recovery);
    }
    length(root, name, SUMMARY_BYTES)?;
    let mut file = open(root, name, true)?;
    file.set_len(0).map_err(|_| Fault::Storage)?;
    file.write_all(bytes).map_err(|_| Fault::Storage)?;
    file.sync_all().map_err(|_| Fault::Storage)
}
pub(super) fn mark_pending(root: &Path) -> Result<(), Fault> {
    rewrite(root, STAGE, b"{}\n")?;
    sync_dir(root)
}
pub(super) fn commit(root: &Path, history: &History) -> Result<(), Fault> {
    rewrite(
        root,
        SUMMARY,
        &serde_json::to_vec(history).map_err(|_| Fault::Recovery)?,
    )?;
    sync_dir(root)?;
    remove(root, STAGE)?;
    sync_dir(root)
}
pub(super) fn remove(root: &Path, name: &str) -> Result<(), Fault> {
    if read(root, name, super::SEGMENT_BYTES)?.is_some() {
        std::fs::remove_file(root.join(name)).map_err(|_| Fault::Storage)?;
    }
    Ok(())
}
pub(super) fn rename(root: &Path, from: &str, to: &str) -> Result<(), Fault> {
    open(root, from, false)?;
    std::fs::rename(root.join(from), root.join(to)).map_err(|_| Fault::Storage)?;
    sync_dir(root)
}
pub(super) fn truncate(root: &Path, name: &str, len: usize) -> Result<(), Fault> {
    let file = open(root, name, true)?;
    file.set_len(len as u64).map_err(|_| Fault::Storage)?;
    file.sync_all().map_err(|_| Fault::Storage)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn dropping_lock_releases_flock_before_a_cloned_descriptor_drops() {
        let root = super::super::tests::root();
        let held = lock(root.path()).unwrap();
        let clone = held._file.try_clone().unwrap();
        drop(held);
        let fresh = lock(root.path()).unwrap();
        drop(fresh);
        super::super::append(
            root.path(),
            super::super::Stream::ToolEvents,
            br#"{"event":"ok"}"#,
        )
        .unwrap();
        drop(clone);
    }
}
