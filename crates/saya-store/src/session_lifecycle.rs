//! Session capacity reporting ahead of the save ceiling: how full a
//! session record is, measured with the same accounting a real save
//! applies — redaction, continuation preparation, then bounded
//! pretty-JSON serialization — without writing anything and without
//! materializing the record into a buffer. [`FsSessionStore::capacity_of`]
//! measures a record in memory; [`FsSessionStore::stored_capacity`]
//! reports an on-disk record's size from its file length.
//!
//! Two-phase deletion lives in the child modules: [`tombstone`] holds the
//! durable record written before anything is removed, and [`delete`] holds
//! the no-follow removal that reports [`DeleteStatus::Pending`] until every
//! path is gone.

pub(crate) mod delete;
pub(crate) mod tombstone;

pub use delete::DeleteStatus;

use crate::bounded::BoundedWriter;
use crate::filesystem::FsSessionStore;
use crate::private_file::io_error;
use crate::redaction::redact;
use crate::{MAX_SESSION_BYTES, RedactedSession, StoreError};
use std::{fs, io::ErrorKind};

/// The warning threshold: a record emitting at or above this many bytes
/// is still savable — the ceiling is [`MAX_SESSION_BYTES`] — but a caller
/// should steer toward session rotation.
pub const SESSION_WARN_BYTES: usize = 3 << 20;

/// How full a session record is relative to the save ceiling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionCapacityStatus {
    /// Below the warning threshold.
    Ok,
    /// At or above the warning threshold and at or below the ceiling:
    /// still savable.
    Warning,
    /// More than the ceiling: a save of this record is refused.
    OverCap,
}

/// The measured size of a session record beside its thresholds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionCapacity {
    /// The emitted pretty-JSON bytes — exactly what a save would write.
    /// For a measured [`SessionCapacityStatus::OverCap`] this is
    /// `max_bytes + 1`: "more than the ceiling", never a count beyond the
    /// bound. For a stored file it is the file's length.
    pub used_bytes: usize,
    /// The warning threshold the status was classified against.
    pub warn_bytes: usize,
    /// The save ceiling the status was classified against.
    pub max_bytes: usize,
    pub status: SessionCapacityStatus,
}

/// The save's prepare step, shared with the capacity report so the two
/// cannot drift: message and turn redaction, then continuation
/// preparation (its own prose redaction plus revalidation).
pub(crate) fn prepare_session_for_persist(session: &mut RedactedSession) -> Result<(), StoreError> {
    for message in &mut session.messages {
        message.content = redact(&message.content);
    }
    for turn in &mut session.turns {
        turn.user = redact(&turn.user);
        turn.assistant = redact(&turn.assistant);
    }
    if let Some(continuation) = &mut session.continuation {
        continuation.prepare_for_persist()?;
    }
    Ok(())
}

/// Classify an emitted byte count against the crate thresholds.
pub(crate) fn capacity_status(used_bytes: usize) -> SessionCapacityStatus {
    if used_bytes > MAX_SESSION_BYTES {
        SessionCapacityStatus::OverCap
    } else if used_bytes >= SESSION_WARN_BYTES {
        SessionCapacityStatus::Warning
    } else {
        SessionCapacityStatus::Ok
    }
}

/// The pretty-JSON bytes a save would emit for the prepared session,
/// counted through a bounded writer over a sink: nothing is written
/// anywhere and the record is never materialized into a buffer. Refusal
/// at the bound reports `max + 1` — over the ceiling, uncounted beyond
/// it.
fn emitted_persist_bytes(session: &RedactedSession) -> Result<usize, StoreError> {
    let mut capped = BoundedWriter::new(std::io::sink(), MAX_SESSION_BYTES);
    match serde_json::to_writer_pretty(&mut capped, session) {
        Ok(()) => Ok(capped.written()),
        Err(error) if error.io_error_kind() == Some(ErrorKind::QuotaExceeded) => {
            Ok(MAX_SESSION_BYTES.saturating_add(1))
        }
        Err(_) => Err(StoreError::unavailable()),
    }
}

/// Measure a record with the save's exact accounting: prepare it, then
/// count the emitted bytes. A continuation that fails preparation
/// returns the same error a save would.
pub(crate) fn measure_capacity(session: RedactedSession) -> Result<SessionCapacity, StoreError> {
    let mut session = session;
    prepare_session_for_persist(&mut session)?;
    let used_bytes = emitted_persist_bytes(&session)?;
    Ok(SessionCapacity {
        used_bytes,
        warn_bytes: SESSION_WARN_BYTES,
        max_bytes: MAX_SESSION_BYTES,
        status: capacity_status(used_bytes),
    })
}

impl FsSessionStore {
    /// How full `session` would be on disk, with the save's exact
    /// accounting. Nothing is written and the record is never
    /// materialized into a buffer.
    pub fn capacity_of(&self, session: RedactedSession) -> Result<SessionCapacity, StoreError> {
        measure_capacity(session)
    }

    /// The size of the session record stored on disk, from the file's
    /// metadata length — which equals the bytes its save emitted. `None`
    /// when no record exists; a file larger than the ceiling, which a
    /// save cannot produce, reports [`SessionCapacityStatus::OverCap`]
    /// with its real length.
    pub fn stored_capacity(&self, id: &str) -> Result<Option<SessionCapacity>, StoreError> {
        let path = self.path(id)?;
        let len = match fs::metadata(&path) {
            Ok(meta) => usize::try_from(meta.len()).map_err(|_| StoreError::unavailable())?,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(io_error(error)),
        };
        Ok(Some(SessionCapacity {
            used_bytes: len,
            warn_bytes: SESSION_WARN_BYTES,
            max_bytes: MAX_SESSION_BYTES,
            status: capacity_status(len),
        }))
    }
}
