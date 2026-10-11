//! Before-images: verified copies of the user's original bytes captured before the first write to a file.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use saya_types::Sha256Digest;

use super::transaction_journal::{JournalError, sha_hex};
use crate::workspace::{MAX_IO_BYTES, Workspace};

/// The maximum total number of before-image bytes one journal's `before/` directory may hold.
pub(super) const APPLY_BACK_IMAGE_TOTAL_BYTES: u64 = 256 * 1024 * 1024;

/// Verified copies of the user's original bytes, stored as digest-named files inside the journal's
/// `before/` directory.
pub(super) struct BeforeImageStore {
    dir: PathBuf,
    total_bytes: u64,
}

impl std::fmt::Debug for BeforeImageStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BeforeImageStore")
            .field("total_bytes", &self.total_bytes)
            .finish()
    }
}

impl BeforeImageStore {
    /// Points the store at the `before/` subdirectory of `journal_dir`, starting with an empty budget.
    pub(super) fn new(journal_dir: &Path) -> Self {
        Self {
            dir: journal_dir.join("before"),
            total_bytes: 0,
        }
    }

    /// Captures the current contents of `rel` from `workspace`, verifies them against `expected`,
    /// and stores a private copy under the before-image directory.
    pub(super) fn capture(
        &mut self,
        workspace: &Workspace,
        rel: &str,
        expected: &Sha256Digest,
    ) -> Result<String, JournalError> {
        let read = workspace
            .read(rel, MAX_IO_BYTES as u64)
            .map_err(|error| match error {
                crate::HarnessError::BoundsExceeded { .. } => JournalError::OverBudget,
                _ => JournalError::Io,
            })?;
        if read.truncated {
            return Err(JournalError::OverBudget);
        }
        if read.digest != expected.as_str() {
            return Err(JournalError::BeforeImageMismatch);
        }
        let len = read.bytes.len() as u64;
        let new_total = self
            .total_bytes
            .checked_add(len)
            .filter(|total| *total <= APPLY_BACK_IMAGE_TOTAL_BYTES)
            .ok_or(JournalError::OverBudget)?;
        let image_path = self.dir.join(sha_hex(rel.as_bytes()));
        let mut file = open_new_image(&image_path)?;
        file.write_all(&read.bytes).map_err(|_| JournalError::Io)?;
        file.sync_all().map_err(|_| JournalError::Io)?;
        fs::File::open(&self.dir)
            .and_then(|d| d.sync_all())
            .map_err(|_| JournalError::Io)?;
        self.total_bytes = new_total;
        Ok(read.digest)
    }

    /// Returns the bytes previously captured for `rel`.
    pub(super) fn load(&self, rel: &str, expected: &Sha256Digest) -> Result<Vec<u8>, JournalError> {
        let store = Workspace::open(&self.dir).map_err(|_| JournalError::Io)?;
        let read = store
            .read(&sha_hex(rel.as_bytes()), MAX_IO_BYTES as u64)
            .map_err(|_| JournalError::Io)?;
        if read.truncated || read.digest != expected.as_str() {
            return Err(JournalError::BeforeImageMismatch);
        }
        Ok(read.bytes)
    }
}

#[cfg(unix)]
fn open_new_image(path: &Path) -> Result<fs::File, JournalError> {
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|_| JournalError::Io)
}

#[cfg(not(unix))]
fn open_new_image(path: &Path) -> Result<fs::File, JournalError> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| JournalError::Io)
}
