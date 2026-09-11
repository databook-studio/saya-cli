//! The run event journal: an append-only `events.ndjson` in the run
//! directory, one [`RunEvent`] per line — the durable record a resume
//! replays.
//!
//! Every event passes [`redact`] before a byte is written: the journal is
//! persisted data, so a credential-shaped string in a payload is written
//! redacted, never raw. Redaction runs on each payload string through the
//! event's JSON value tree — not on the serialized line — so the redacted
//! bytes stay valid NDJSON a resume can parse.
//!
//! A torn trailing line (a crash mid-append) is not a complete line and is
//! ignored on read: the unit it may have recorded is re-executed on resume
//! rather than guessed at. A *complete* line that fails to parse is
//! corruption and fails closed with [`HarnessError::JournalCorrupt`].

use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};

use saya_types::{RunEvent, redact};
use serde_json::Value;

use crate::{HarnessError, io_error};

/// The journal file's name inside a run directory.
pub const EVENTS_FILE: &str = "events.ndjson";

/// An observer fired once per successfully appended event, with the event as
/// journaled. The headless run wire (`saya-cli`) attaches one that renders
/// the event onto the process's NDJSON stream, so the wire and the durable
/// record are the same stream in the same order — the renderer cannot drift
/// from the journal because it renders the journal's own write.
pub type JournalWire = Arc<dyn Fn(&RunEvent) + Send + Sync>;

/// The append-only event journal of one run.
#[derive(Clone)]
pub struct Journal {
    path: PathBuf,
    wire: Option<JournalWire>,
}

impl std::fmt::Debug for Journal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Journal")
            .field("path", &self.path)
            .field("wire", &self.wire.is_some())
            .finish()
    }
}

impl Journal {
    /// The journal of the run directory `run_dir` (`runs/<id>/`). The file
    /// is created by the first append, never by opening.
    pub fn open(run_dir: impl AsRef<Path>) -> Self {
        Self {
            path: run_dir.as_ref().join(EVENTS_FILE),
            wire: None,
        }
    }

    /// Attaches the observer every subsequent append notifies. Clones share
    /// the observer, so a journal handed to several writers reports each
    /// event exactly once.
    pub fn with_wire(mut self, wire: JournalWire) -> Self {
        self.wire = Some(wire);
        self
    }

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

    /// Every event in the journal, in write order. An absent journal is an
    /// empty one.
    pub fn read(&self) -> Result<Vec<RunEvent>, HarnessError> {
        let raw = match fs::read_to_string(&self.path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(io_error("read run journal", &self.path, error)),
        };
        parse_lines(&raw).map_err(|line| HarnessError::JournalCorrupt { line })
    }

    /// The state a resume starts from, replayed from the journal.
    pub fn rebuild(&self) -> Result<JournalState, HarnessError> {
        Ok(replay(&self.read()?))
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

/// The state a resume starts from, rebuilt by replaying the journal in
/// write order. Steps the plan bound but the journal never mentions are
/// absent here — the engine holds the plan, so "first incomplete step"
/// stays its question to ask.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct JournalState {
    /// A `RunStarted` is recorded.
    pub started: bool,
    /// A `PlanApproved` is recorded.
    pub plan_approved: bool,
    /// Every step the journal mentions, keyed by its plan index. Last write
    /// wins: a step that failed and was started again by the bounded retry
    /// reads back as [`StepState::Started`].
    pub steps: BTreeMap<usize, StepState>,
    /// The journal's last lifecycle event — a pause's reason or a terminal
    /// state lives here. Usage reports are deliberately not lifecycle.
    pub last: Option<RunEvent>,
}

/// Where a step stands, as the journal last recorded it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepState {
    /// Its episode began; it may be mid-flight or awaiting a bounded retry.
    Started,
    Completed,
    Failed,
}

/// Replays journal events into the state a resume starts from.
pub fn replay(events: &[RunEvent]) -> JournalState {
    let mut state = JournalState::default();
    for event in events {
        // Every lifecycle event is a candidate for `last`, not only the ones
        // without a state-tracking arm of their own. Setting it inside the
        // catch-all meant a journal ending on StepStarted reported no last
        // event at all — precisely the shape a resume reads.
        if !matches!(event, RunEvent::Usage { .. }) {
            state.last = Some(event.clone());
        }
        match event {
            RunEvent::RunStarted => state.started = true,
            RunEvent::PlanApproved => state.plan_approved = true,
            RunEvent::StepStarted { step } => {
                state.steps.insert(*step, StepState::Started);
            }
            RunEvent::StepCompleted { step } => {
                state.steps.insert(*step, StepState::Completed);
            }
            RunEvent::StepFailed { step } => {
                state.steps.insert(*step, StepState::Failed);
            }
            RunEvent::Usage { .. } => {}
            _ => {}
        }
    }
    state
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

/// Parses every newline-terminated line. A torn final line — no newline —
/// was never a complete event and is ignored; a complete line that does not
/// parse fails closed with its 1-based line number.
fn parse_lines(raw: &str) -> Result<Vec<RunEvent>, usize> {
    let torn_tail = !raw.is_empty() && !raw.ends_with('\n');
    let total = raw.lines().count();
    let mut events = Vec::new();
    for (index, line) in raw.lines().enumerate() {
        if line.is_empty() || (torn_tail && index + 1 == total) {
            continue;
        }
        events.push(serde_json::from_str(line).map_err(|_| index + 1)?);
    }
    Ok(events)
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
