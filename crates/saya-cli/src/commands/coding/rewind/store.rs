//! On-disk checkpoint storage for one job: one private directory per turn
//! under `<job records>/rewind/`, holding that checkpoint's record and the
//! exact bytes of each path it recorded.
//!
//! A capture writes the images first and the record last, so a checkpoint that
//! exists is always complete: a crash between the two leaves a turn directory
//! with no record, which the listing ignores and the next capture of that turn
//! replaces. Nothing here is redacted — a faithful restore needs the exact
//! bytes — and nothing here is ever advertised as authority.

use super::{
    CapturedPath, MAX_REWIND_CHECKPOINT_BYTES, MAX_REWIND_CHECKPOINTS, MAX_REWIND_FILE_BYTES,
    MAX_REWIND_FILES_PER_CHECKPOINT, MAX_REWIND_RECORD_BYTES, RewindCheckpoint, RewindEntry,
};
use crate::commands::coding::{CodingError, context, host};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const VERSION: u32 = 1;
const RECORD_NAME: &str = "record.json";
const NOTE_NAME: &str = "last-rewind.json";
const FAILURE_NAME: &str = "capture-failure.json";

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u32,
    turn: u32,
    created_unix_ms: i64,
    entries: Vec<RecordEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RecordEntry {
    path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sha256: Option<String>,
    size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    image: Option<String>,
}

/// The durable note that the checkout was rewound, read and cleared by the
/// next run so its first turn's context states what happened.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RewindNote {
    version: u32,
    turn: u32,
    at_unix_ms: i64,
}

/// The durable note that one turn boundary could not be checkpointed: a closed
/// class, so the operator learns a checkpoint is missing without the store
/// ever holding an error's own text.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CaptureFailure {
    version: u32,
    turn: u32,
    class: String,
}

/// One job's checkpoint store. `open` performs no I/O: the directory is created
/// by the first capture, so listing a job that never checkpointed creates
/// nothing.
pub(crate) struct RewindStore {
    root: PathBuf,
}

impl RewindStore {
    pub(crate) fn open(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
        }
    }

    fn turn_dir(&self, turn: u32) -> PathBuf {
        self.root.join(format!("turn-{turn:04}"))
    }

    /// Every checkpoint's number, ascending. A directory without a readable
    /// record is not a checkpoint and is not listed.
    pub(crate) fn turns(&self) -> Result<Vec<u32>, CodingError> {
        let entries = match std::fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(CodingError::owner(error)),
        };
        let mut turns = Vec::new();
        for entry in entries {
            let entry = entry.map_err(CodingError::owner)?;
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let Some(digits) = name.strip_prefix("turn-") else {
                continue;
            };
            let Ok(turn) = digits.parse::<u32>() else {
                continue;
            };
            if self.turn_dir(turn).join(RECORD_NAME).is_file() {
                turns.push(turn);
            }
        }
        turns.sort_unstable();
        Ok(turns)
    }

    /// The number the next checkpoint takes: one past the highest recorded.
    pub(crate) fn next_turn(&self) -> Result<u32, CodingError> {
        Ok(self.turns()?.last().copied().unwrap_or(0).saturating_add(1))
    }

    pub(crate) fn list(&self) -> Result<Vec<RewindCheckpoint>, CodingError> {
        self.turns()?
            .into_iter()
            .map(|turn| {
                let record = self.read_record(turn)?;
                Ok(RewindCheckpoint {
                    turn: record.turn,
                    created_unix_ms: record.created_unix_ms,
                    entries: record
                        .entries
                        .into_iter()
                        .map(|entry| RewindEntry {
                            path: entry.path,
                            sha256: entry.sha256,
                            size: entry.size,
                        })
                        .collect(),
                })
            })
            .collect()
    }

    /// One checkpoint's recorded paths and bytes. A recorded path whose image
    /// is missing or unreadable is a corrupt checkpoint, never a silent gap.
    pub(crate) fn images(&self, turn: u32) -> Result<Vec<CapturedPath>, CodingError> {
        let record = self.read_record(turn)?;
        let dir = self.turn_dir(turn);
        record
            .entries
            .into_iter()
            .map(|entry| {
                let bytes = match entry.image {
                    None => None,
                    Some(image) => Some(context::read(
                        &dir.join(&image),
                        MAX_REWIND_FILE_BYTES as usize,
                    )?),
                };
                Ok(CapturedPath {
                    path: entry.path,
                    bytes,
                })
            })
            .collect()
    }

    /// Records one checkpoint, or nothing at all. Every bound is checked
    /// before the first byte is written, so a refused capture leaves no turn
    /// directory behind and the listing never shows a partial state. The
    /// oldest checkpoint is dropped first once the job holds
    /// [`MAX_REWIND_CHECKPOINTS`].
    pub(crate) fn capture(
        &self,
        turn: u32,
        captured: &[CapturedPath],
        now_ms: i64,
    ) -> Result<(), CodingError> {
        if captured.len() > MAX_REWIND_FILES_PER_CHECKPOINT {
            return Err(CodingError::RewindCaptureTooLarge);
        }
        let mut total = 0u64;
        for path in captured {
            let size = path.bytes.as_ref().map_or(0, Vec::len) as u64;
            if size > MAX_REWIND_FILE_BYTES {
                return Err(CodingError::RewindCaptureTooLarge);
            }
            total = total.saturating_add(size);
            if total > MAX_REWIND_CHECKPOINT_BYTES {
                return Err(CodingError::RewindCaptureTooLarge);
            }
        }
        let mut record = Record {
            version: VERSION,
            turn,
            created_unix_ms: now_ms,
            entries: Vec::with_capacity(captured.len()),
        };
        for (index, path) in captured.iter().enumerate() {
            let image = path.bytes.as_ref().map(|_| format!("{index:04}.bin"));
            record.entries.push(RecordEntry {
                path: path.path.clone(),
                sha256: path
                    .bytes
                    .as_ref()
                    .map(|bytes| saya_types::sha256_hex(bytes)),
                size: path.bytes.as_ref().map_or(0, Vec::len) as u64,
                image,
            });
        }
        let encoded = serde_json::to_vec(&record).map_err(CodingError::owner)?;
        if encoded.len() > MAX_REWIND_RECORD_BYTES {
            return Err(CodingError::RewindCaptureTooLarge);
        }
        let dir = host::private_directory(&self.turn_dir(turn))?;
        for (entry, path) in record.entries.iter().zip(captured) {
            if let (Some(image), Some(bytes)) = (&entry.image, path.bytes.as_ref()) {
                context::write_new(&dir.join(image), bytes, MAX_REWIND_FILE_BYTES as usize)?;
            }
        }
        context::write_new(&dir.join(RECORD_NAME), &encoded, MAX_REWIND_RECORD_BYTES)?;
        self.prune()?;
        Ok(())
    }

    /// Drops the oldest checkpoints until the job holds the bound.
    fn prune(&self) -> Result<(), CodingError> {
        let turns = self.turns()?;
        let excess = turns.len().saturating_sub(MAX_REWIND_CHECKPOINTS);
        for turn in turns.into_iter().take(excess) {
            let dir = self.turn_dir(turn);
            std::fs::remove_dir_all(&dir).map_err(CodingError::owner)?;
        }
        Ok(())
    }

    fn read_record(&self, turn: u32) -> Result<Record, CodingError> {
        let bytes = context::read(
            &self.turn_dir(turn).join(RECORD_NAME),
            MAX_REWIND_RECORD_BYTES,
        )?;
        let record: Record = serde_json::from_slice(&bytes).map_err(CodingError::owner)?;
        if record.version != VERSION || record.turn != turn {
            return Err(CodingError::InvalidRequest);
        }
        Ok(record)
    }

    /// Records that the checkout was rewound to `turn`, for the next run to
    /// state in its context.
    pub(crate) fn record_rewind(&self, turn: u32, now_ms: i64) -> Result<(), CodingError> {
        let dir = host::private_directory(&self.root)?;
        let note = RewindNote {
            version: VERSION,
            turn,
            at_unix_ms: now_ms,
        };
        let bytes = serde_json::to_vec(&note).map_err(CodingError::owner)?;
        let path = dir.join(NOTE_NAME);
        // The note is a single slot: the newest rewind replaces the previous
        // one, so an unread older note can never outlive it.
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(CodingError::owner(error)),
        }
        context::write_new(&path, &bytes, MAX_REWIND_RECORD_BYTES)
    }

    /// Takes the pending rewind note, if any, so exactly one run states it.
    /// An unreadable note is dropped rather than reported: it is a note about
    /// the past, never authority over the next turn.
    pub(crate) fn take_rewind_note(&self) -> Option<u32> {
        let path = self.root.join(NOTE_NAME);
        let bytes = context::read(&path, MAX_REWIND_RECORD_BYTES).ok()?;
        let note: RewindNote = serde_json::from_slice(&bytes).ok()?;
        if note.version != VERSION {
            return None;
        }
        let _ = std::fs::remove_file(&path);
        Some(note.turn)
    }

    /// Records that the turn boundary `turn` could not be checkpointed, so the
    /// listing says a checkpoint is missing instead of leaving the gap silent.
    /// The newest failure replaces the previous one.
    pub(crate) fn record_capture_failure(
        &self,
        turn: u32,
        class: &'static str,
    ) -> Result<(), CodingError> {
        let dir = host::private_directory(&self.root)?;
        let note = CaptureFailure {
            version: VERSION,
            turn,
            class: class.to_owned(),
        };
        let bytes = serde_json::to_vec(&note).map_err(CodingError::owner)?;
        let path = dir.join(FAILURE_NAME);
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(CodingError::owner(error)),
        }
        context::write_new(&path, &bytes, MAX_REWIND_RECORD_BYTES)
    }

    /// Clears the capture-failure note once a turn is checkpointed again: the
    /// gap it named is not the newest state.
    pub(crate) fn clear_capture_failure(&self) -> Result<(), CodingError> {
        match std::fs::remove_file(self.root.join(FAILURE_NAME)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(CodingError::owner(error)),
        }
    }

    /// The last capture failure, as the listing reports it.
    pub(crate) fn capture_failure(&self) -> Option<(u32, String)> {
        let bytes = context::read(&self.root.join(FAILURE_NAME), MAX_REWIND_RECORD_BYTES).ok()?;
        let note: CaptureFailure = serde_json::from_slice(&bytes).ok()?;
        (note.version == VERSION).then_some((note.turn, note.class))
    }
}
