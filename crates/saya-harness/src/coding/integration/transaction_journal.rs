//! The apply-back journal: an append-only, checksummed, fsynced local record
//! of per-file progress. The line format and the read side live in
//! `transaction_journal_io.rs`; before-images live in `transaction_images.rs`.

use std::fs::{self, File, OpenOptions};
use std::num::NonZeroU64;
use std::path::{Path, PathBuf};

use saya_types::JobId;
use sha2::{Digest, Sha256};

pub(super) use super::transaction_images::BeforeImageStore;

/// Why the journal or the before-image store refused. Displays are static.
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum JournalError {
    #[error("the apply-back journal is unusable")]
    Io,
    #[error("a journal record is corrupt")]
    Corrupt,
    #[error("the before-image budget is exceeded")]
    OverBudget,
    #[error("a before-image digest did not match the baseline")]
    BeforeImageMismatch,
}

/// How a journaled file changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum JournalKind {
    Added,
    Modified,
    Deleted,
}

/// One journal record: an intent (written after the before-image exists) or
/// the matching done. Digests are lowercase-hex strings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum JournalEvent {
    Begin {
        binding: String,
        baseline_modes: std::collections::BTreeMap<String, u32>,
    },
    Intent {
        path: String,
        kind: JournalKind,
        baseline: Option<String>,
        delta: Option<String>,
        image: Option<String>,
    },
    Done {
        path: String,
        result: Option<String>,
    },
    Restoring {
        path: String,
    },
    Restored {
        path: String,
    },
}

/// An open journal: its directory and the append handle.
pub(super) struct ApplyBackJournal {
    pub(super) dir: PathBuf,
    pub(super) file: File,
}

impl std::fmt::Debug for ApplyBackJournal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ApplyBackJournal { .. }")
    }
}

/// Lowercase hex SHA-256 of `bytes`.
pub(super) fn sha_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

pub(super) fn job_dir(state_dir: &Path, job_id: &JobId) -> PathBuf {
    state_dir
        .join("coding")
        .join("apply-back")
        .join(job_id.as_str())
}

impl ApplyBackJournal {
    /// Creates `<state_dir>/coding/apply-back/<job>/<operation_sequence>/`
    /// (private 0700) with its `before/` directory and a fresh 0600
    /// `journal.jsonl`, fsyncing the directory.
    pub(super) fn create(
        state_dir: &Path,
        job_id: &JobId,
        operation_sequence: NonZeroU64,
    ) -> Result<Self, JournalError> {
        let dir = job_dir(state_dir, job_id).join(operation_sequence.get().to_string());
        let parent = dir.parent().ok_or(JournalError::Io)?;
        fs::create_dir_all(parent).map_err(|_| JournalError::Io)?;
        make_private_dir(&dir)?;
        make_private_dir(&dir.join("before"))?;
        let file = open_journal(&dir.join("journal.jsonl"))?;
        File::open(&dir)
            .and_then(|handle| handle.sync_all())
            .map_err(|_| JournalError::Io)?;
        if !dir.starts_with(state_dir) {
            return Err(JournalError::Io);
        }
        for ancestor in dir.ancestors() {
            File::open(ancestor)
                .and_then(|h| h.sync_all())
                .map_err(|_| JournalError::Io)?;
            if ancestor == state_dir {
                break;
            }
        }
        Ok(Self { dir, file })
    }

    /// The journal's directory (the before-images live under `before/`).
    pub(super) fn dir(&self) -> &Path {
        &self.dir
    }
}

#[cfg(unix)]
fn make_private_dir(path: &Path) -> Result<(), JournalError> {
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new()
        .mode(0o700)
        .create(path)
        .map_err(|_| JournalError::Io)
}

#[cfg(not(unix))]
fn make_private_dir(path: &Path) -> Result<(), JournalError> {
    fs::create_dir(path).map_err(|_| JournalError::Io)
}

#[cfg(unix)]
fn open_journal(path: &Path) -> Result<File, JournalError> {
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|_| JournalError::Io)
}

#[cfg(not(unix))]
fn open_journal(path: &Path) -> Result<File, JournalError> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| JournalError::Io)
}

/// The highest numeric operation-sequence subdirectory of
/// `<state_dir>/coding/apply-back/<job_id>/`, or `None`.
pub fn apply_back_latest_dir(state_dir: &Path, job_id: &JobId) -> Option<PathBuf> {
    let entries = fs::read_dir(job_dir(state_dir, job_id)).ok()?;
    entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let sequence: u64 = entry.file_name().to_str()?.parse().ok()?;
            Some((sequence, entry.path()))
        })
        .max_by_key(|(sequence, _)| *sequence)
        .map(|(_, path)| path)
}
