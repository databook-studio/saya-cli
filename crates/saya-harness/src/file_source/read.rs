//! The contained read of the staging source: the source path's parent is
//! opened as a [`Workspace`] solely to open that one final component
//! no-follow with a post-open identity check — nothing else in the parent is
//! listed or opened. Symlinks, non-regular files, and files over the 32 MiB
//! CSV ceiling are refused by that contained open; the sha256 is computed
//! over the bytes read in the same single open.

use std::{ffi::OsStr, path::Path};

use crate::{HarnessError, workspace::Workspace};

use super::StageError;

/// The source file's once-read content and its identity, as staging consumes it.
pub(super) struct SourceRead {
    pub(super) bytes: Vec<u8>,
    pub(super) sha256: String,
    pub(super) file_name: String,
    pub(super) stem: String,
    pub(super) size: u64,
}

pub(super) fn read_source(source: &Path) -> Result<SourceRead, StageError> {
    let invalid = || {
        StageError::Source(HarnessError::InvalidPath {
            path: source.display().to_string(),
        })
    };
    let file_name = source
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or_else(invalid)?;
    let stem = source
        .file_stem()
        .and_then(OsStr::to_str)
        .unwrap_or(file_name);
    let parent = source
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let workspace = Workspace::open(parent).map_err(StageError::Source)?;
    let read = workspace
        .read_for_scratch_import(file_name)
        .map_err(StageError::Source)?;
    Ok(SourceRead {
        bytes: read.bytes,
        sha256: read.digest,
        file_name: file_name.to_owned(),
        stem: stem.to_owned(),
        size: read.size,
    })
}
