//! The `SessionJournal` handle: journal-once write methods, the append and
//! read core, and the settled wire form each event line is written under.

use super::events::{BypassSource, GrantSource, JournalEvent};
use super::files::{bounded_read, io_error, open_append, parse_lines, truncate_torn_tail};
use super::{JOURNAL_FILE, MAX_JOURNAL_BYTES};
use crate::StoreError;
use crate::redaction::redact;
use std::{fs, io::Write, path::Path, path::PathBuf};

/// The append-only event journal of one session. One writer at a time: the
/// state directory is single-writer by the session lock, and the journal is
/// opened once per process, under that lock.
#[derive(Clone, Debug)]
pub struct SessionJournal {
    path: PathBuf,
}

impl SessionJournal {
    /// The journal of the session state directory `state_dir`
    /// (`sessions/<id>/`), opened once per process under the session lock.
    /// The file is created by the first write, never by opening; a torn
    /// trailing line a crash left is dropped here, so the next write cannot
    /// concatenate onto it and turn a journal that reads into one that
    /// corrupts (the run journal's own rule).
    pub fn open(state_dir: impl AsRef<Path>) -> Self {
        let path = state_dir.as_ref().join(JOURNAL_FILE);
        truncate_torn_tail(&path);
        Self { path }
    }

    /// Journals one token grant, redacted at write through the repo's existing
    /// [`saya_types::redact`] seam — never a second rule set. The caller
    /// owns the journal-once rule: it writes only for a grant the store
    /// reports as new.
    pub fn granted(&self, token: &str, source: GrantSource) -> Result<(), StoreError> {
        self.append(&JournalEvent::Granted {
            token: redact(token),
            source,
        })
    }

    /// Journals one bypass activation. No payload beyond the source: bypass
    /// grants no token, so the line carries no token-shaped secret.
    pub fn bypass_activated(&self, source: BypassSource) -> Result<(), StoreError> {
        self.append(&JournalEvent::Bypass { source })
    }

    /// Journals the session's deny list once at session start, when
    /// non-empty: the denied programs, sorted. No noise when empty.
    pub fn deny_list(&self, programs: &[String]) -> Result<(), StoreError> {
        if programs.is_empty() {
            return Ok(());
        }
        self.append(&JournalEvent::DenyList {
            programs: programs.iter().map(|program| redact(program)).collect(),
        })
    }

    /// Journals one host call — the program named in the ask, its argv, and
    /// the door the ask entered through — redacted at write through the
    /// repo's existing seam, never a second rule set. Written before the
    /// child spawns: journal-before-spawn.
    pub fn command(&self, program: &str, argv: &[String], door: &str) -> Result<(), StoreError> {
        self.append(&JournalEvent::Command {
            program: redact(program),
            argv: argv.iter().map(|arg| redact(arg)).collect(),
            door: door.to_owned(),
        })
    }

    /// Journals one deny firing — the program named in the ask, its argv,
    /// and the door the ask entered through — redacted at write through the
    /// repo's existing seam, never a second rule set. Written before the
    /// refusal is relayed: journal-before-refusal.
    pub fn command_denied(
        &self,
        program: &str,
        argv: &[String],
        door: &str,
    ) -> Result<(), StoreError> {
        self.append(&JournalEvent::CommandDenied {
            program: redact(program),
            argv: argv.iter().map(|arg| redact(arg)).collect(),
            door: door.to_owned(),
        })
    }

    /// Appends one event as exactly one newline-terminated NDJSON line and
    /// fsyncs it. The line bytes go through order-preserving struct
    /// serialization — serde's internally-tagged buffering would reorder the
    /// keys alphabetically, and the settled shape is byte-pinned.
    pub(super) fn append(&self, event: &JournalEvent) -> Result<(), StoreError> {
        let line = match event {
            JournalEvent::Granted { token, source } => serde_json::to_string(&GrantedLine {
                event: "session-granted",
                token,
                source: *source,
            }),
            JournalEvent::Bypass { source } => serde_json::to_string(&BypassLine {
                event: "session-bypass",
                source: *source,
            }),
            JournalEvent::DenyList { programs } => serde_json::to_string(&DenyListLine {
                event: "session-deny-list",
                programs,
            }),
            JournalEvent::CommandDenied {
                program,
                argv,
                door,
            } => serde_json::to_string(&CommandDeniedLine {
                event: "session-command-denied",
                program,
                argv,
                door,
            }),
            JournalEvent::Command {
                program,
                argv,
                door,
            } => serde_json::to_string(&CommandLine {
                event: "session-command",
                program,
                argv,
                door,
            }),
        }
        .map_err(|_| StoreError::Invalid)?;
        let append_bytes = line.len().saturating_add(1);
        let existing_bytes = match fs::metadata(&self.path) {
            Ok(metadata) => metadata.len(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
            Err(_) => return Err(StoreError::Unavailable),
        };
        if append_bytes > MAX_JOURNAL_BYTES
            || existing_bytes > (MAX_JOURNAL_BYTES - append_bytes) as u64
        {
            return Err(StoreError::LimitExceeded);
        }
        let mut file = open_append(&self.path)?;
        if file.metadata().map_err(|_| StoreError::Unavailable)?.len()
            > (MAX_JOURNAL_BYTES - append_bytes) as u64
        {
            return Err(StoreError::LimitExceeded);
        }
        file.write_all(line.as_bytes())
            .and_then(|()| file.write_all(b"\n"))
            .map_err(|error| io_error(&self.path, error))?;
        file.sync_all()
            .map_err(|error| io_error(&self.path, error))?;
        Ok(())
    }

    /// Every event in the journal, in write order. An absent journal is an
    /// empty one. A torn trailing line — a crash mid-append — is not a
    /// complete event and is ignored on read; a complete line that fails to
    /// parse is corruption and fails closed.
    pub fn read(&self) -> Result<Vec<JournalEvent>, StoreError> {
        let raw = match bounded_read(&self.path)? {
            Some(bytes) => String::from_utf8(bytes).map_err(|_| StoreError::Invalid)?,
            None => return Ok(Vec::new()),
        };
        parse_lines(&raw)
    }
}

/// The written form of one grant line, in the settled key order (`event`,
/// then `token`, then `source`) — a struct serializes in declaration order.
#[derive(serde::Serialize)]
struct GrantedLine<'a> {
    event: &'static str,
    token: &'a str,
    source: GrantSource,
}

/// The written form of one bypass line, in the settled key order.
#[derive(serde::Serialize)]
struct BypassLine {
    event: &'static str,
    source: BypassSource,
}

/// The written form of the session-start deny list, in the settled key
/// order (`event`, then `programs`).
#[derive(serde::Serialize)]
struct DenyListLine<'a> {
    event: &'static str,
    programs: &'a [String],
}

/// The written form of one deny firing, in the settled key order (`event`,
/// then `program`, `argv`, `door`).
#[derive(serde::Serialize)]
struct CommandDeniedLine<'a> {
    event: &'static str,
    program: &'a str,
    argv: &'a [String],
    door: &'a str,
}

/// The written form of one host call, in the settled key order (`event`,
/// then `program`, `argv`, `door`).
#[derive(serde::Serialize)]
struct CommandLine<'a> {
    event: &'static str,
    program: &'a str,
    argv: &'a [String],
    door: &'a str,
}
