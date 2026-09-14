//! The session journal: an append-only `journal.ndjson` in the session's
//! state directory (`sessions/<id>/`), one event per line. It is the audit
//! record of what the user consented to — never a grant source: nothing
//! reads it back into a grant store, so a resumed session starts empty.

use crate::StoreError;
use crate::redaction::redact;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

/// The journal file's name inside a session state directory. Reserved since
/// U1 (`session_paths.rs` in `saya-cli`); written from U7 on.
pub const JOURNAL_FILE: &str = "journal.ndjson";

/// Why a token was granted — the `source` field of a `session-granted` line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrantSource {
    /// A token seeded by `/allow`, journaled when it is seeded — before
    /// anything runs under it.
    Seed,
    /// A token granted by answering `[s]` at an ask, journaled when the
    /// grant lands — before the call it first allowed runs.
    Prompt,
}

/// What activated bypass — the `source` field of a `session-bypass` line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BypassSource {
    /// The launch stated the mode: a fresh session under its launch mode, or
    /// a resume whose `--approval-mode` explicitly overrode the record.
    Launch,
    /// `/approvals bypass` flipped the mode mid-session.
    Command,
}

/// One journal line, tagged on `event`, so each line is exactly the settled
/// shape: `{"event":"session-granted","token":…,"source":…}` and
/// `{"event":"session-bypass","source":…}`. Serialization writes the settled
/// key order (see `append`); deserialization accepts the tag anywhere.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(tag = "event")]
#[non_exhaustive]
pub enum JournalEvent {
    #[serde(rename = "session-granted")]
    Granted { token: String, source: GrantSource },
    #[serde(rename = "session-bypass")]
    Bypass { source: BypassSource },
    /// The session's deny list, once at session start when non-empty: the
    /// denied programs, sorted, in the order the list carries them.
    #[serde(rename = "session-deny-list")]
    DenyList { programs: Vec<String> },
    /// One host call: the program named in the ask and its argv, on the door
    /// the ask entered through. Redacted through the existing seam, written
    /// before the child spawns — consent-before-action for the one lane
    /// where it matters most.
    #[serde(rename = "session-command")]
    Command {
        program: String,
        argv: Vec<String>,
        door: String,
    },
    /// One deny firing: the program named in the ask, its argv, and the door
    /// the ask entered through. Redacted through the existing seam, written
    /// before the refusal is relayed — journal-before-spawn's mirror.
    #[serde(rename = "session-command-denied")]
    CommandDenied {
        program: String,
        argv: Vec<String>,
        door: String,
    },
}

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
    fn append(&self, event: &JournalEvent) -> Result<(), StoreError> {
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
        let mut file = open_append(&self.path)?;
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
        let raw = match fs::read_to_string(&self.path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(io_error(&self.path, error)),
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

/// Drops a torn trailing line — the half-written event a crash left — so a
/// write cannot concatenate onto it. A whole-line journal is untouched, and
/// an absent journal creates nothing.
fn truncate_torn_tail(path: &Path) {
    let Ok(raw) = fs::read_to_string(path) else {
        return;
    };
    if raw.is_empty() || raw.ends_with('\n') {
        return;
    }
    let keep = raw.rfind('\n').map_or(0, |index| index + 1);
    if let Ok(mut file) = OpenOptions::new().write(true).truncate(true).open(path) {
        let _ = file.write_all(&raw.as_bytes()[..keep]);
        let _ = file.sync_all();
    }
}

/// Parses every newline-terminated line. A torn final line — no newline —
/// was never a complete event and is ignored; a complete line that does not
/// parse fails closed.
fn parse_lines(raw: &str) -> Result<Vec<JournalEvent>, StoreError> {
    let torn_tail = !raw.is_empty() && !raw.ends_with('\n');
    let total = raw.lines().count();
    raw.lines()
        .enumerate()
        .filter(|(index, line)| !line.is_empty() && !(torn_tail && index + 1 == total))
        .map(|(_, line)| serde_json::from_str(line).map_err(|_| StoreError::Invalid))
        .collect()
}

/// Opens the journal for appending, creating it 0600 — a session state dir
/// holds no readable-by-others bytes.
fn open_append(path: &Path) -> Result<fs::File, StoreError> {
    // `mode` is an extension trait, so the import is cfg-gated with its use —
    // importing it unconditionally does not compile on Windows.
    #[cfg(unix)]
    use std::os::unix::fs::OpenOptionsExt;

    let mut options = OpenOptions::new();
    options.append(true).create(true);
    #[cfg(unix)]
    options.mode(0o600);
    options.open(path).map_err(|error| io_error(path, error))
}

/// `StoreError` is payload-free by contract, so an `io::Error` can only map
/// to the store's own unavailable shape.
fn io_error(_: &Path, _: std::io::Error) -> StoreError {
    StoreError::Unavailable
}

#[cfg(test)]
#[path = "session_journal_tests.rs"]
mod tests;
