//! The durable write path of the journal: the fsynced, redacted append and
//! the torn-tail repair. Every event passes the redaction tree before a
//! byte lands, and a crash's half-written line is dropped so a resume's
//! appends cannot concatenate onto it.

use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
};

use saya_types::{RunEvent, redact};
use serde_json::Value;

use crate::{HarnessError, io_error};

use super::Journal;

impl Journal {
    /// Appends one event as exactly one newline-terminated NDJSON line and
    /// fsyncs it — the journal is the record a crash resume reads. A
    /// successful append fires the observer (after the write, so an observer
    /// never announces an event that failed to land).
    pub fn append(&self, event: &RunEvent) -> Result<(), HarnessError> {
        let line = serialize_redacted(event)?;
        let mut file = open_append(&self.path)?;
        file.write_all(line.as_bytes())
            .and_then(|()| file.write_all(b"\n"))
            .map_err(|error| io_error("append run event to", &self.path, error))?;
        file.sync_all()
            .map_err(|error| io_error("sync run journal", &self.path, error))?;
        if let Some(wire) = &self.wire {
            wire(event);
        }
        Ok(())
    }

    /// Drops a torn trailing line — the half-written event a crash left —
    /// so a resume's appends cannot concatenate onto it and turn a journal
    /// that reads into one that corrupts. A whole-line journal is
    /// untouched; the run lock must be held.
    pub fn truncate_torn_tail(&self) -> Result<(), HarnessError> {
        let raw = match fs::read_to_string(&self.path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(io_error("read run journal", &self.path, error)),
        };
        if raw.is_empty() || raw.ends_with('\n') {
            return Ok(());
        }
        let keep = raw.rfind('\n').map_or(0, |index| index + 1);
        let mut file = OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&self.path)
            .map_err(|error| io_error("truncate torn journal tail", &self.path, error))?;
        file.write_all(&raw.as_bytes()[..keep])
            .and_then(|()| file.sync_all())
            .map_err(|error| io_error("truncate torn journal tail", &self.path, error))
    }
}

/// Serializes the event to one NDJSON line, redacting every payload string.
fn serialize_redacted(event: &RunEvent) -> Result<String, HarnessError> {
    let value =
        serde_json::to_value(event).map_err(|source| HarnessError::JournalEncode { source })?;
    serde_json::to_string(&redact_value(value))
        .map_err(|source| HarnessError::JournalEncode { source })
}

/// Redacts string values, recursively. Object keys are structure, not
/// payload, and pass untouched.
fn redact_value(value: Value) -> Value {
    match value {
        Value::String(text) => Value::String(redact(&text)),
        Value::Array(items) => Value::Array(items.into_iter().map(redact_value).collect()),
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(key, value)| (key, redact_value(value)))
                .collect(),
        ),
        other => other,
    }
}

/// Opens the journal for appending, creating it 0600 — a run dir holds no
/// readable-by-others bytes.
fn open_append(path: &Path) -> Result<fs::File, HarnessError> {
    // `mode` is an extension trait, so the import is cfg-gated with its use —
    // importing it unconditionally does not compile on Windows.
    #[cfg(unix)]
    use std::os::unix::fs::OpenOptionsExt;

    let mut options = OpenOptions::new();
    options.append(true).create(true);
    #[cfg(unix)]
    options.mode(0o600);
    options
        .open(path)
        .map_err(|error| io_error("open run journal", path, error))
}
