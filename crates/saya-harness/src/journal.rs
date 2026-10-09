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

#[cfg(doc)]
use crate::HarnessError;
#[cfg(doc)]
use saya_types::redact;

mod append;
mod replay;

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use saya_types::RunEvent;

pub use replay::{JournalState, StepState, replay};

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
}
