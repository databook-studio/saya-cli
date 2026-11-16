//! Bounded diagnostic history. These files carry no execution authority.
mod io;
#[cfg(all(test, unix))]
mod process_tests;
mod state;
#[cfg(all(test, unix))]
mod tests;

use serde::{Deserialize, Serialize};
use state::{Snapshot, load};
use std::{io::Write, path::Path};

pub(super) const SEGMENT_BYTES: usize = 1024 * 1024;
pub(super) const RECORD_BYTES: usize = 8 * 1024;
const SUMMARY_BYTES: usize = 16 * 1024;
const MAX_RECORDS: usize = 65_536;
const LOCK: &str = ".diagnostics.lock";
const SUMMARY: &str = "diagnostics-summary.json";
const STAGE: &str = "diagnostics-summary.stage";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, thiserror::Error)]
#[serde(rename_all = "snake_case")]
pub enum Fault {
    #[error("invalid diagnostic record")]
    InvalidRecord,
    #[error("unsafe diagnostic path")]
    UnsafePath,
    #[error("diagnostic lock contended")]
    LockContended,
    #[error("diagnostic storage failed")]
    Storage,
    #[error("diagnostic recovery failed")]
    Recovery,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Stream {
    ToolEvents,
    Compactions,
    Observations,
}
impl Stream {
    const ALL: [Self; 3] = [Self::ToolEvents, Self::Compactions, Self::Observations];
    fn name(self) -> &'static str {
        match self {
            Self::ToolEvents => "tool-events.jsonl",
            Self::Compactions => "compactions.jsonl",
            Self::Observations => "observations.jsonl",
        }
    }
    fn previous(self) -> &'static str {
        match self {
            Self::ToolEvents => "tool-events.previous.jsonl",
            Self::Compactions => "compactions.previous.jsonl",
            Self::Observations => "observations.previous.jsonl",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum HistoryState {
    Complete,
    Truncated,
    Unknown,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct Segment {
    bytes: u64,
    records: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StreamHistory {
    stream: Stream,
    generation: Option<u64>,
    previous_generation: Option<u64>,
    evicted_records: Option<u64>,
    evicted_bytes: Option<u64>,
    current: Segment,
    previous: Option<Segment>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct History {
    schema_version: u32,
    pub(super) state: HistoryState,
    streams: Vec<StreamHistory>,
}

/// Normal capacity pressure rotates whole records. Real storage faults stay fatal.
pub(super) fn append(records: &Path, stream: Stream, bytes: &[u8]) -> Result<(), Fault> {
    validate_record(bytes)?;
    let _lock = io::lock(records)?;
    let mut snapshot = load(records, false)?;
    io::mark_pending(records)?;
    let index = Stream::ALL
        .iter()
        .position(|item| *item == stream)
        .ok_or(Fault::InvalidRecord)?;
    let history = &mut snapshot.history.streams[index];
    if history.current.bytes + bytes.len() as u64 + 1 > SEGMENT_BYTES as u64
        || history.current.records >= MAX_RECORDS as u64
    {
        if let Some(previous) = &history.previous {
            history.evicted_records = history
                .evicted_records
                .and_then(|value| value.checked_add(previous.records));
            history.evicted_bytes = history
                .evicted_bytes
                .and_then(|value| value.checked_add(previous.bytes));
        }
        io::remove(records, stream.previous())?;
        io::rename(records, stream.name(), stream.previous())?;
        history.previous = Some(std::mem::take(&mut history.current));
        history.previous_generation = history.generation;
        history.generation = history.generation.and_then(|value| value.checked_add(1));
    }
    let mut file = io::open(records, stream.name(), true)?;
    file.write_all(bytes).map_err(|_| Fault::Storage)?;
    file.write_all(b"\n").map_err(|_| Fault::Storage)?;
    file.sync_all().map_err(|_| Fault::Storage)?;
    history.current.bytes += bytes.len() as u64 + 1;
    history.current.records += 1;
    snapshot.history.refresh();
    io::commit(records, &snapshot.history)
}

pub(super) fn history(records: &Path) -> Result<History, Fault> {
    let _lock = io::lock(records)?;
    Ok(load(records, true)?.history)
}

/// Export a single locked snapshot in retained generation order.
pub(super) fn export(records: &Path) -> Result<serde_json::Value, Fault> {
    let _lock = io::lock(records)?;
    let Snapshot { history, files } = load(records, true)?;
    Ok(
        serde_json::json!({"history":history,"files":files,"ordering":"previous_then_current",
        "active_file_alone_is_full_history":false}),
    )
}
fn validate_record(bytes: &[u8]) -> Result<(), Fault> {
    if bytes.len() + 1 > RECORD_BYTES || bytes.contains(&b'\n') || bytes.is_empty() {
        return Err(Fault::InvalidRecord);
    }
    if !serde_json::from_slice::<serde_json::Value>(bytes)
        .map_err(|_| Fault::InvalidRecord)?
        .is_object()
    {
        return Err(Fault::InvalidRecord);
    }
    Ok(())
}
impl History {
    fn refresh(&mut self) {
        self.state = if self.streams.iter().any(|stream| {
            stream.evicted_records.is_none()
                || stream.evicted_bytes.is_none()
                || stream.generation.is_none()
        }) {
            HistoryState::Unknown
        } else if self
            .streams
            .iter()
            .any(|stream| stream.evicted_records.is_some_and(|count| count > 0))
        {
            HistoryState::Truncated
        } else {
            HistoryState::Complete
        };
    }
}
