//! The bounded, private before-image store for one in-place job.
//!
//! Objects are written before the index line that names them, so a crash
//! leaves an orphan object and never an index entry without bytes. A bound
//! records an explicit `Unrecorded` reason and sets `overflowed`; it never
//! refuses an edit and never returns an error.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::coding::snapshot::sensitive_path;
use saya_types::WorkspacePath;

use super::index::{self, Event};
use super::objects::{digest, private_directory, read_object, write_object};
use super::{
    BeforeImageEntry, BeforeState, FirstTouch, MAX_BEFORE_IMAGE_FILE_BYTES, MAX_BEFORE_IMAGE_PATHS,
    MAX_BEFORE_IMAGE_TOTAL_BYTES, UnrecordedReason,
};

/// A before-image store failure. Bounds are not errors; only I/O and corrupt
/// stored data are.
#[derive(Debug, thiserror::Error)]
pub enum BeforeImageError {
    #[error("before-image store I/O failed")]
    Io(#[from] std::io::Error),
    #[error("before-image index is malformed")]
    Corrupt,
    #[error("before-image object does not match its recorded digest")]
    DigestMismatch,
}

/// One job's before-image store. A single mutex covers the index and its
/// in-memory view, which the job's root and helpers share.
pub struct BeforeImageStore {
    objects: PathBuf,
    index: PathBuf,
    inner: Mutex<Inner>,
    overflowed: AtomicBool,
    temp_counter: AtomicU64,
}

struct Inner {
    entries: BTreeMap<String, BeforeImageEntry>,
    total_bytes: u64,
}

impl fmt::Debug for BeforeImageStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let count = self.inner.lock().map_or(0, |inner| inner.entries.len());
        formatter
            .debug_struct("BeforeImageStore")
            .field("entries", &count)
            .field("overflowed", &self.overflowed())
            .finish_non_exhaustive()
    }
}

impl BeforeImageStore {
    /// Opens the store under `<job_records>/before-images/`, creating it
    /// private when absent.
    pub fn open(job_records: &Path) -> Result<Self, BeforeImageError> {
        let root = job_records.join("before-images");
        private_directory(&root)?;
        let objects = root.join("objects");
        private_directory(&objects)?;
        let index = root.join("index.jsonl");
        let entries = index::load(&index)?;
        let total_bytes = entries
            .values()
            .filter_map(|entry| match entry.state {
                BeforeState::File { size, .. } => Some(size),
                _ => None,
            })
            .fold(0u64, u64::saturating_add);
        let overflowed = entries.values().any(|entry| {
            matches!(
                entry.state,
                BeforeState::Unrecorded(
                    UnrecordedReason::OverFileBound
                        | UnrecordedReason::OverTotalBound
                        | UnrecordedReason::OverPathBound
                )
            )
        });
        Ok(Self {
            objects,
            index,
            inner: Mutex::new(Inner {
                entries,
                total_bytes,
            }),
            overflowed: AtomicBool::new(overflowed),
            temp_counter: AtomicU64::new(0),
        })
    }

    /// Records the first bytes the job saw at `path`. An earlier touch always
    /// wins; a bound records an explicit reason and never refuses the edit.
    pub fn record_first_touch(
        &self,
        path: &WorkspacePath,
        before: Option<&[u8]>,
        executable: bool,
        turn: u32,
    ) -> Result<FirstTouch, BeforeImageError> {
        let mut inner = self.lock()?;
        if inner.entries.contains_key(path.as_str()) {
            return Ok(FirstTouch::AlreadyRecorded);
        }
        if inner.entries.len() >= MAX_BEFORE_IMAGE_PATHS {
            self.mark_overflowed();
            return Ok(FirstTouch::Unrecorded(UnrecordedReason::OverPathBound));
        }
        let state = if sensitive_path(path.as_str()) {
            BeforeState::Unrecorded(UnrecordedReason::Sensitive)
        } else {
            match before {
                Some(bytes) if bytes.len() as u64 > MAX_BEFORE_IMAGE_FILE_BYTES => {
                    BeforeState::Unrecorded(UnrecordedReason::OverFileBound)
                }
                Some(bytes)
                    if inner.total_bytes.saturating_add(bytes.len() as u64)
                        > MAX_BEFORE_IMAGE_TOTAL_BYTES =>
                {
                    BeforeState::Unrecorded(UnrecordedReason::OverTotalBound)
                }
                Some(bytes) => {
                    let digest = digest(bytes)?;
                    BeforeState::File {
                        sha256: digest,
                        size: bytes.len() as u64,
                        executable,
                    }
                }
                None => BeforeState::Absent,
            }
        };
        let event = Event::Touch {
            path: path.as_str().to_owned(),
            state: state.clone(),
            turn,
        };
        let line = event.line()?;
        if !index::fits(&self.index, line.len())? {
            self.mark_overflowed();
            return Ok(FirstTouch::Unrecorded(UnrecordedReason::OverPathBound));
        }
        // The object lands before the index line that names it.
        if let BeforeState::File { sha256, .. } = &state {
            write_object(
                &self.objects,
                &self.temp_counter,
                sha256,
                before.unwrap_or_default(),
            )?;
        }
        if !index::append(&self.index, &line)? {
            self.mark_overflowed();
            return Ok(FirstTouch::Unrecorded(UnrecordedReason::OverPathBound));
        }
        if let BeforeState::File { size, .. } = state {
            inner.total_bytes = inner.total_bytes.saturating_add(size);
        }
        let reason = match &state {
            BeforeState::Unrecorded(reason) => Some(*reason),
            _ => None,
        };
        inner.entries.insert(
            path.as_str().to_owned(),
            BeforeImageEntry {
                path: path.as_str().to_owned(),
                state,
                turn,
            },
        );
        if let Some(reason) = reason {
            if reason != UnrecordedReason::Sensitive {
                self.mark_overflowed();
            }
            return Ok(FirstTouch::Unrecorded(reason));
        }
        Ok(FirstTouch::Recorded)
    }

    /// Drops a first touch whose write provably did not land.
    pub fn forget(&self, path: &str) -> Result<(), BeforeImageError> {
        let mut inner = self.lock()?;
        if !inner.entries.contains_key(path) {
            return Ok(());
        }
        let line = Event::Forget {
            path: path.to_owned(),
        }
        .line()?;
        if !index::append(&self.index, &line)? {
            index::compact(&self.index, &inner.entries)?;
            if !index::append(&self.index, &line)? {
                return Err(BeforeImageError::Io(std::io::Error::other(
                    "before-image index is full",
                )));
            }
        }
        if let Some(entry) = inner.entries.remove(path)
            && let BeforeState::File { size, .. } = entry.state
        {
            inner.total_bytes = inner.total_bytes.saturating_sub(size);
        }
        Ok(())
    }

    /// Every latest record, sorted by path.
    pub fn entries(&self) -> Result<Vec<BeforeImageEntry>, BeforeImageError> {
        Ok(self.lock()?.entries.values().cloned().collect())
    }

    /// The stored bytes for one recorded entry, re-verified against its
    /// digest. An absent or deliberately unrecorded path has no image.
    pub fn read_image(
        &self,
        entry: &BeforeImageEntry,
    ) -> Result<Option<Vec<u8>>, BeforeImageError> {
        let BeforeState::File { sha256, size, .. } = &entry.state else {
            return Ok(None);
        };
        let bytes = read_object(&self.objects.join(sha256.as_str()))?;
        if bytes.len() as u64 != *size || digest(&bytes)? != *sha256 {
            return Err(BeforeImageError::DigestMismatch);
        }
        Ok(Some(bytes))
    }

    /// Whether any bound was reached for this store.
    pub fn overflowed(&self) -> bool {
        self.overflowed.load(Ordering::Acquire)
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Inner>, BeforeImageError> {
        self.inner.lock().map_err(|_| {
            BeforeImageError::Io(std::io::Error::other("before-image store is poisoned"))
        })
    }

    fn mark_overflowed(&self) {
        self.overflowed.store(true, Ordering::Release);
    }
}
