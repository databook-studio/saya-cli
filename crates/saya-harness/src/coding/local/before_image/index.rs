//! The append-only event index behind one before-image store: one bounded
//! JSON line per touch or forget, the latest record per path winning.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{Read as _, Write as _};
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::{BeforeImageEntry, BeforeImageError, BeforeState, MAX_BEFORE_IMAGE_INDEX_BYTES};

/// One durable index event.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub(super) enum Event {
    Touch {
        path: String,
        state: BeforeState,
        turn: u32,
    },
    Forget {
        path: String,
    },
}

impl Event {
    pub(super) fn line(&self) -> Result<Vec<u8>, BeforeImageError> {
        serde_json::to_vec(self).map_err(|_| BeforeImageError::Corrupt)
    }

    fn apply(self, entries: &mut BTreeMap<String, BeforeImageEntry>) {
        match self {
            Self::Touch { path, state, turn } => {
                entries.insert(path.clone(), BeforeImageEntry { path, state, turn });
            }
            Self::Forget { path } => {
                entries.remove(&path);
            }
        }
    }
}

/// Reads every complete event. A torn final line is ignored; any earlier
/// malformed line is corruption, never a silently weaker state.
pub(super) fn load(path: &Path) -> Result<BTreeMap<String, BeforeImageEntry>, BeforeImageError> {
    let mut entries = BTreeMap::new();
    let bytes = match read_bounded(path)? {
        Some(bytes) => bytes,
        None => return Ok(entries),
    };
    let mut lines: Vec<&[u8]> = bytes.split(|byte| *byte == b'\n').collect();
    if lines.last().is_some_and(|line| line.is_empty()) {
        lines.pop();
    }
    let last = lines.len().saturating_sub(1);
    for (index, line) in lines.iter().enumerate() {
        if line.is_empty() {
            continue;
        }
        match serde_json::from_slice::<Event>(line) {
            Ok(event) => event.apply(&mut entries),
            Err(_) if index == last => break,
            Err(_) => return Err(BeforeImageError::Corrupt),
        }
    }
    Ok(entries)
}

/// Appends one event unless the index would pass its byte bound. The caller
/// holds the store's lock, so the size check and the append are atomic within
/// the process.
pub(super) fn append(path: &Path, line: &[u8]) -> Result<bool, BeforeImageError> {
    if !fits(path, line.len())? {
        return Ok(false);
    }
    let mut options = OpenOptions::new();
    options.append(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    let mut file = options.open(path)?;
    file.write_all(line)?;
    file.write_all(b"\n")?;
    file.sync_data()?;
    Ok(true)
}

/// Whether one more line fits. A missing index has no bytes yet.
pub(super) fn fits(path: &Path, line_len: usize) -> Result<bool, BeforeImageError> {
    let current = match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() => metadata.len(),
        Ok(_) => return Err(BeforeImageError::Corrupt),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
        Err(error) => return Err(error.into()),
    };
    Ok(current.saturating_add(line_len as u64 + 1) <= MAX_BEFORE_IMAGE_INDEX_BYTES as u64)
}

/// Rewrites the index to the current latest-per-path state, so a forget can
/// still be appended once the append-only history has reached the byte bound.
pub(super) fn compact(
    path: &Path,
    entries: &BTreeMap<String, BeforeImageEntry>,
) -> Result<(), BeforeImageError> {
    let mut bytes = Vec::new();
    for entry in entries.values() {
        let event = Event::Touch {
            path: entry.path.clone(),
            state: entry.state.clone(),
            turn: entry.turn,
        };
        bytes.extend_from_slice(&event.line()?);
        bytes.push(b'\n');
    }
    if bytes.len() > MAX_BEFORE_IMAGE_INDEX_BYTES {
        return Err(BeforeImageError::Corrupt);
    }
    let temp = path.with_extension("jsonl.tmp");
    match fs::remove_file(&temp) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
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
    file.write_all(&bytes)?;
    file.sync_all()?;
    fs::rename(&temp, path)?;
    Ok(())
}

/// Reads the index without following a symlink, bounded before and after the
/// read. A missing index is an empty history.
fn read_bounded(path: &Path) -> Result<Option<Vec<u8>>, BeforeImageError> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    let mut file = match options.open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > MAX_BEFORE_IMAGE_INDEX_BYTES as u64 {
        return Err(BeforeImageError::Corrupt);
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    if bytes.len() > MAX_BEFORE_IMAGE_INDEX_BYTES {
        return Err(BeforeImageError::Corrupt);
    }
    Ok(Some(bytes))
}
