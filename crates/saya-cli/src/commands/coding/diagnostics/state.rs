//! Recovery never infers precise loss or order from a partially committed write.
use super::{
    Fault, History, HistoryState, MAX_RECORDS, SEGMENT_BYTES, STAGE, SUMMARY, SUMMARY_BYTES,
    Segment, Stream, StreamHistory, io, validate_record,
};
use std::path::Path;
pub(super) struct Snapshot {
    pub(super) history: History,
    pub(super) files: Vec<serde_json::Value>,
}
pub(super) fn load(root: &Path, include_records: bool) -> Result<Snapshot, Fault> {
    let pending = io::read(root, STAGE, SUMMARY_BYTES)?.is_some();
    let saved = io::read(root, SUMMARY, SUMMARY_BYTES)?;
    let saved = if pending {
        None
    } else {
        saved
            .as_deref()
            .map(serde_json::from_slice::<History>)
            .transpose()
            .map_err(|_| Fault::Recovery)?
    };
    if let Some(saved) = &saved
        && (saved.schema_version != 1
            || saved.streams.len() != Stream::ALL.len()
            || saved
                .streams
                .iter()
                .zip(Stream::ALL)
                .any(|(entry, stream)| entry.stream != stream))
    {
        return Err(Fault::Recovery);
    }
    let mut files = Vec::new();
    let mut streams = Vec::new();
    for (index, stream) in Stream::ALL.into_iter().enumerate() {
        let saved_stream = saved.as_ref().map(|saved| &saved.streams[index]);
        let fast = !include_records && !pending;
        let (previous, previous_file, repaired_previous) = segment(
            root,
            stream.previous(),
            saved_stream.and_then(|entry| entry.previous.as_ref()),
            fast,
        )?;
        let (current, current_file, repaired_current) = segment(
            root,
            stream.name(),
            saved_stream.map(|entry| &entry.current),
            fast,
        )?;
        let exists = previous.is_some() || current.is_some();
        let current = current.unwrap_or_default();
        let saved = saved.as_ref().map(|saved| &saved.streams[index]);
        let known = !pending
            && !repaired_previous
            && !repaired_current
            && saved.is_none_or(|saved| saved.current == current && saved.previous == previous)
            && (saved.is_some() || !exists);
        let mut history = saved.cloned().unwrap_or(StreamHistory {
            stream,
            generation: Some(0),
            previous_generation: None,
            evicted_records: Some(0),
            evicted_bytes: Some(0),
            current: Segment::default(),
            previous: None,
        });
        if !known {
            history.generation = None;
            history.previous_generation = None;
            history.evicted_records = None;
            history.evicted_bytes = None;
        }
        history.current = current;
        history.previous = previous;
        streams.push(history);
        files.extend(previous_file);
        files.extend(current_file);
    }
    let mut history = History {
        schema_version: 1,
        state: HistoryState::Complete,
        streams,
    };
    history.refresh();
    Ok(Snapshot { history, files })
}
fn segment(
    root: &Path,
    name: &str,
    expected: Option<&Segment>,
    fast: bool,
) -> Result<(Option<Segment>, Option<serde_json::Value>, bool), Fault> {
    if fast
        && let Some(expected) = expected
        && io::length(root, name, SEGMENT_BYTES)? == Some(expected.bytes)
    {
        if expected.records > MAX_RECORDS as u64 {
            return Err(Fault::Recovery);
        }
        return Ok((Some(expected.clone()), None, false));
    }
    let Some(mut bytes) = io::read(root, name, SEGMENT_BYTES)? else {
        return Ok((None, None, false));
    };
    let repaired = !bytes.is_empty() && !bytes.ends_with(b"\n");
    if repaired {
        let end = bytes
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |position| position + 1);
        // A whole final record without its framing is still an interrupted tail.
        // Mark uncertainty durably before removing only those incomplete bytes.
        io::mark_pending(root)?;
        io::truncate(root, name, end)?;
        bytes.truncate(end);
    }
    let mut records = Vec::new();
    for framed in bytes.split_inclusive(|byte| *byte == b'\n') {
        let line = &framed[..framed.len() - 1];
        validate_record(line).map_err(|_| Fault::Recovery)?;
        if records.len() >= MAX_RECORDS {
            return Err(Fault::Recovery);
        }
        records
            .push(serde_json::from_slice::<serde_json::Value>(line).map_err(|_| Fault::Recovery)?);
    }
    let segment = Segment {
        bytes: bytes.len() as u64,
        records: records.len() as u64,
    };
    Ok((
        Some(segment),
        Some(serde_json::json!({"name":name,"records":records})),
        repaired,
    ))
}
