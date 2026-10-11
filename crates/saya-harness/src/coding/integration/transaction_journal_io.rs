//! The journal's line format and read side. Each line is one compact JSON
//! object with a final `sum` field: the SHA-256 of the object's compact text
//! WITHOUT `sum`. A torn or corrupt tail fails its checksum and is ignored:
//! the valid prefix is the truth.

use std::io::Write;
use std::path::Path;

use serde_json::{Value, json};

use super::transaction_journal::{
    ApplyBackJournal, JournalError, JournalEvent, JournalKind, sha_hex,
};

fn kind_name(kind: JournalKind) -> &'static str {
    match kind {
        JournalKind::Added => "added",
        JournalKind::Modified => "modified",
        JournalKind::Deleted => "deleted",
    }
}

fn kind_from(name: &str) -> Option<JournalKind> {
    match name {
        "added" => Some(JournalKind::Added),
        "modified" => Some(JournalKind::Modified),
        "deleted" => Some(JournalKind::Deleted),
        _ => None,
    }
}

impl ApplyBackJournal {
    /// Appends one checksummed line and fsyncs it.
    pub(super) fn append(&mut self, event: &JournalEvent) -> Result<(), JournalError> {
        let mut object = match event {
            JournalEvent::Intent {
                path,
                kind,
                baseline,
                delta,
                image,
            } => json!({
                "event": "intent",
                "path": path,
                "kind": kind_name(*kind),
                "baseline": baseline,
                "delta": delta,
                "image": image,
            }),
            JournalEvent::Done { path, result } => json!({
                "event": "done",
                "path": path,
                "result": result,
            }),
        };
        let text = serde_json::to_string(&object).map_err(|_| JournalError::Io)?;
        object["sum"] = Value::String(sha_hex(text.as_bytes()));
        let line = serde_json::to_string(&object).map_err(|_| JournalError::Io)?;
        self.file
            .write_all(line.as_bytes())
            .map_err(|_| JournalError::Io)?;
        self.file.write_all(b"\n").map_err(|_| JournalError::Io)?;
        self.file.sync_all().map_err(|_| JournalError::Io)
    }

    /// Reads `journal.jsonl` in `dir` and returns the events up to the first
    /// line that fails to parse or whose checksum does not match.
    #[allow(dead_code)] // consumed by W11b recovery
    pub(super) fn read_valid_prefix(dir: &Path) -> Result<Vec<JournalEvent>, JournalError> {
        let bytes = std::fs::read(dir.join("journal.jsonl")).map_err(|_| JournalError::Io)?;
        let mut events = Vec::new();
        for chunk in bytes.split(|b| *b == b'\n') {
            if chunk.is_empty() {
                continue;
            }
            match std::str::from_utf8(chunk).ok().and_then(parse_line) {
                Some(event) => events.push(event),
                None => break,
            }
        }
        Ok(events)
    }
}

fn parse_line(line: &str) -> Option<JournalEvent> {
    let mut value: Value = serde_json::from_str(line).ok()?;
    let sum = value.as_object_mut()?.remove("sum")?.as_str()?.to_owned();
    let text = serde_json::to_string(&value).ok()?;
    if sha_hex(text.as_bytes()) != sum {
        return None;
    }
    let object = value.as_object()?;
    let required = |name: &str| object.get(name)?.as_str().map(str::to_owned);
    let optional = |name: &str| match object.get(name)? {
        Value::Null => Some(None),
        Value::String(text) => Some(Some(text.clone())),
        _ => None,
    };
    match object.get("event")?.as_str()? {
        "intent" => Some(JournalEvent::Intent {
            path: required("path")?,
            kind: kind_from(object.get("kind")?.as_str()?)?,
            baseline: optional("baseline")?,
            delta: optional("delta")?,
            image: optional("image")?,
        }),
        "done" => Some(JournalEvent::Done {
            path: required("path")?,
            result: optional("result")?,
        }),
        _ => None,
    }
}
