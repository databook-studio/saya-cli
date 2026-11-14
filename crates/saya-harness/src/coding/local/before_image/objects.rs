//! The byte-object side of a before-image store: content-addressed files,
//! their no-follow reads, and the private directories that hold them.

use std::fs::{self, OpenOptions};
use std::io::{Read as _, Write as _};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use saya_types::Sha256Digest;

use super::{BeforeImageError, MAX_BEFORE_IMAGE_FILE_BYTES};

/// Writes one object under its digest, via a temp name and a rename. The
/// caller holds the store's lock, so the temp counter cannot collide.
pub(super) fn write_object(
    objects: &Path,
    temp_counter: &AtomicU64,
    sha256: &Sha256Digest,
    bytes: &[u8],
) -> Result<(), BeforeImageError> {
    let path = objects.join(sha256.as_str());
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.is_file() => return Ok(()),
        Ok(_) => {
            return Err(BeforeImageError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "before-image object is not a regular file",
            )));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let temp = objects.join(format!(
        ".tmp-{}-{}",
        std::process::id(),
        temp_counter.fetch_add(1, Ordering::Relaxed)
    ));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    let mut file = options.open(&temp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(&temp, &path)?;
    Ok(())
}

/// Reads one object without following a symlink, bounded before and after
/// the read.
pub(super) fn read_object(path: &Path) -> Result<Vec<u8>, BeforeImageError> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    let mut file = options.open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > MAX_BEFORE_IMAGE_FILE_BYTES {
        return Err(BeforeImageError::DigestMismatch);
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BEFORE_IMAGE_FILE_BYTES {
        return Err(BeforeImageError::DigestMismatch);
    }
    Ok(bytes)
}

pub(super) fn digest(bytes: &[u8]) -> Result<Sha256Digest, BeforeImageError> {
    Sha256Digest::parse(&saya_types::sha256_hex(bytes)).map_err(|_| BeforeImageError::Corrupt)
}

/// Creates the directory private (0700, owned by this user) when absent and
/// refuses one that is not.
pub(super) fn private_directory(path: &Path) -> Result<(), BeforeImageError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, MetadataExt};
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true).mode(0o700);
        match builder.create(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
        let metadata = fs::symlink_metadata(path)?;
        // SAFETY: geteuid reads the process identity and takes no pointer.
        let owner_uid = unsafe { libc::geteuid() };
        if !metadata.is_dir() || metadata.mode() & 0o077 != 0 || metadata.uid() != owner_uid {
            return Err(BeforeImageError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "before-image directory is not private",
            )));
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        fs::create_dir_all(path)?;
        Ok(())
    }
}
