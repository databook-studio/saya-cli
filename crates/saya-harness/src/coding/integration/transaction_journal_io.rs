//! The journal's line format and read side. Each line is one compact JSON
//! object with a final `sum` field: the SHA-256 of the object's compact text
//! WITHOUT `sum`. A torn or corrupt tail fails its checksum and is ignored:
//! the valid prefix is the truth.

use saya_types::JobId;
use std::io::{Read, Seek, SeekFrom, Write};
use std::num::NonZeroU64;

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

pub(super) fn encode_event(event: &JournalEvent) -> Result<Vec<u8>, JournalError> {
    let mut object = match event {
        JournalEvent::Begin {
            binding,
            baseline_modes,
        } => json!({"event": "begin", "binding": binding, "baseline_modes": baseline_modes}),
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
        JournalEvent::Restoring { path } => json!({"event": "restoring", "path": path}),
        JournalEvent::Restored { path } => json!({"event": "restored", "path": path}),
    };
    let text = serde_json::to_string(&object).map_err(|_| JournalError::Io)?;
    object["sum"] = Value::String(sha_hex(text.as_bytes()));
    let mut line = serde_json::to_vec(&object).map_err(|_| JournalError::Io)?;
    line.push(b'\n');
    Ok(line)
}

impl ApplyBackJournal {
    /// Refuses an over-budget append before writing any bytes, then fsyncs.
    pub(super) fn append(&mut self, event: &JournalEvent) -> Result<(), JournalError> {
        let line = encode_event(event)?;
        let length = self.file.metadata().map_err(|_| JournalError::Io)?.len();
        if length
            .checked_add(line.len() as u64)
            .is_none_or(|size| size > super::recover_binding::MAX_RECOVERY_JOURNAL_BYTES)
        {
            return Err(JournalError::OverBudget);
        }
        self.file.write_all(&line).map_err(|_| JournalError::Io)?;
        self.file.sync_all().map_err(|_| JournalError::Io)
    }

    /// Reopens only an existing, bounded regular journal under trusted host state.
    /// Tail truncation happens separately, after semantic replay validates the prefix.
    pub(super) fn reopen(
        state_dir: &Path,
        job: &JobId,
        sequence: NonZeroU64,
    ) -> Result<(Self, Vec<JournalEvent>, u64), JournalError> {
        let dir =
            super::transaction_journal::job_dir(state_dir, job).join(sequence.get().to_string());
        let mut options = std::fs::OpenOptions::new();
        options.read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        let mut file = options
            .open(dir.join("journal.jsonl"))
            .map_err(|_| JournalError::Io)?;
        let meta = file.metadata().map_err(|_| JournalError::Io)?;
        let cap = super::recover_journal::MAX_RECOVERY_JOURNAL_BYTES;
        if !meta.is_file() || meta.len() > cap {
            return Err(JournalError::OverBudget);
        }
        let mut bytes = Vec::new();
        (&mut file)
            .take(cap + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| JournalError::Io)?;
        if bytes.len() as u64 > cap {
            return Err(JournalError::OverBudget);
        }
        let (mut events, mut valid) = (Vec::new(), 0usize);
        for chunk in bytes.split_inclusive(|b| *b == b'\n') {
            if !chunk.ends_with(b"\n") {
                break;
            }
            match std::str::from_utf8(&chunk[..chunk.len() - 1])
                .ok()
                .and_then(parse_line)
            {
                Some(event) => {
                    events.push(event);
                    valid += chunk.len();
                }
                None => break,
            }
        }
        Ok((Self { dir, file }, events, valid as u64))
    }

    pub(super) fn truncate_tail(&mut self, valid: u64) -> Result<(), JournalError> {
        self.file.set_len(valid).map_err(|_| JournalError::Io)?;
        self.file
            .seek(SeekFrom::End(0))
            .map_err(|_| JournalError::Io)?;
        self.file.sync_all().map_err(|_| JournalError::Io)
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
        "begin" => Some(JournalEvent::Begin {
            binding: required("binding")?,
            baseline_modes: serde_json::from_value(object.get("baseline_modes")?.clone()).ok()?,
        }),
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
        "restoring" => Some(JournalEvent::Restoring {
            path: required("path")?,
        }),
        "restored" => Some(JournalEvent::Restored {
            path: required("path")?,
        }),
        _ => None,
    }
}
