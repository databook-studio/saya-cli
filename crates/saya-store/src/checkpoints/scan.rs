//! The bounded directory scan behind listing and usage: one pass over the
//! session's checkpoints directory that classifies every entry by name and
//! reads at most `MAX_CHECKPOINTS_PER_SESSION + 8` record files. Files that
//! fail validation or disagree with their name are ignored — never deleted
//! — and no record the quota counted can be hidden by temps, orphans, or
//! foreign names, which are classified without being read.

use super::files::dir_path;
use super::{CheckpointStoreError, invalid};
use crate::FsSessionStore;
use crate::filesystem::is_valid_id;
use crate::private_file::io_error;
use saya_types::{
    BeforeImage, CheckpointId, CheckpointUsage, EditCheckpoint, MAX_CHECKPOINTS_PER_SESSION,
};
use std::{fs, io::ErrorKind, path::Path};

/// How many record files one scan reads at most: every checkpoint a
/// session can validly hold, plus a margin for invalid or foreign-shaped
/// records.
pub(super) const SCAN_BOUND: usize = MAX_CHECKPOINTS_PER_SESSION as usize + 8;

/// One directory entry name classified against the checkpoint file shapes.
pub(super) enum Class {
    /// `<ck-id>.json` whose stem parses as a checkpoint id.
    Record(CheckpointId),
    /// `<ck-id>.before` whose stem parses as a checkpoint id.
    Image(CheckpointId),
    /// A staging temp: dot-prefixed and `.tmp`-suffixed, the shape the
    /// private staging primitives mint.
    Temp,
    /// Anything else, including well-shaped names that fail to parse.
    Foreign,
}

pub(super) fn classify(name: &str) -> Class {
    if name.starts_with('.') && name.ends_with(".tmp") {
        return Class::Temp;
    }
    if let Some(stem) = name.strip_suffix(".json") {
        return CheckpointId::parse(stem).map_or(Class::Foreign, Class::Record);
    }
    if let Some(stem) = name.strip_suffix(".before") {
        return CheckpointId::parse(stem).map_or(Class::Foreign, Class::Image);
    }
    Class::Foreign
}

/// One bounded scan: the valid records sorted by created time then id,
/// their usage totals, and how many entries were skipped.
pub(super) struct Scan {
    pub records: Vec<EditCheckpoint>,
    pub usage: CheckpointUsage,
    pub skipped: usize,
    examined: usize,
}

impl Scan {
    fn empty() -> Self {
        Self {
            records: Vec::new(),
            usage: CheckpointUsage::default(),
            skipped: 0,
            examined: 0,
        }
    }
}

/// Scans the session's checkpoints directory once. A missing directory
/// scans empty; record files are read up to [`SCAN_BOUND`], and reads that
/// fail validation or identity are counted as skipped, never deleted.
pub(super) fn scan_dir(root: &Path, session_id: &str) -> Result<Scan, CheckpointStoreError> {
    let dir = dir_path(root, session_id);
    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Scan::empty()),
        Err(error) => return Err(CheckpointStoreError::from(io_error(error))),
    };
    let mut scan = Scan::empty();
    for entry in entries.filter_map(Result::ok) {
        let Ok(name) = entry.file_name().into_string() else {
            scan.skipped += 1;
            continue;
        };
        let id = match classify(&name) {
            Class::Record(id) => id,
            _ => continue,
        };
        if scan.examined >= SCAN_BOUND {
            break;
        }
        scan.examined += 1;
        match super::records::read_record_file(&entry.path(), &id, session_id) {
            Ok(record) => {
                scan.usage.edits = scan.usage.edits.saturating_add(1);
                if let BeforeImage::Present(version) = &record.before {
                    scan.usage.retained_bytes =
                        scan.usage.retained_bytes.saturating_add(version.size);
                }
                scan.records.push(record);
            }
            Err(_) => scan.skipped += 1,
        }
    }
    scan.records.sort_by(|left, right| {
        (left.created_unix_ms, left.id.as_str()).cmp(&(right.created_unix_ms, right.id.as_str()))
    });
    Ok(scan)
}

impl FsSessionStore {
    /// Every valid checkpoint of the session, sorted by created time then
    /// id. Files that fail validation or disagree with their name are
    /// ignored, never deleted.
    pub fn list_checkpoints(
        &self,
        session_id: &str,
    ) -> Result<Vec<EditCheckpoint>, CheckpointStoreError> {
        if !is_valid_id(session_id) {
            return Err(invalid());
        }
        Ok(scan_dir(self.sessions_root(), session_id)?.records)
    }

    /// How many valid checkpoints the session holds and how many bytes
    /// their before-images retain. Quota admission reads this; a scan
    /// failure fails closed rather than reporting zero usage.
    pub fn checkpoint_usage(
        &self,
        session_id: &str,
    ) -> Result<CheckpointUsage, CheckpointStoreError> {
        if !is_valid_id(session_id) {
            return Err(invalid());
        }
        Ok(scan_dir(self.sessions_root(), session_id)?.usage)
    }
}
