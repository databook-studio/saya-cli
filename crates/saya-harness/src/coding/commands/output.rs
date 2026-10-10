//! The bounded private output store for owned command sessions.
//!
//! A session started through
//! [`super::lifecycle::CommandOwner::start_with_output`] attaches a store to
//! the run's pump seam: each raw chunk goes to a per-stream sink
//! ([`super::output_sink`]) that hands it to a dedicated writer thread
//! ([`super::output_writer`]), which redacts it through the runner's
//! two-pass gate — with a holdback window so a secret split across pump
//! reads is never persisted in fragments — and appends the redacted bytes
//! to a per-stream file under a crate-generated directory. No
//! user-controlled path ever names a store entry: the root is the caller's
//! record-dir lineage, the leaf is a `{nanos}-{counter}` nonce, and the
//! per-stream files carry fixed names.
//!
//! The store is append-only head-keep: redacted bytes are appended until
//! the cap, then one ASCII truncation marker line is written and the file
//! stops growing — the head window stays byte-stable for later cursor reads
//! (C02b), while the final `ProgramOutcome` ring keeps the tail. There is
//! no re-scrub pass: the honest accounting is reported through
//! [`CommandOutputStatus`], failures typed `(kind, raw os code)`, no text.
//!
//! Directory and file creation is descriptor-relative and no-follow on
//! Unix: every existing directory the store is pointed at must already be
//! owned by the effective user with group and other access cleared and is
//! refused — never chmodded — otherwise; everything the store creates is
//! 0700/0600 by construction, and a pre-placed symlink is refused, never
//! adopted. That descriptor/privacy seam lives in
//! [`super::output_store_unix`]. The store is Unix-only: platforms without
//! the `openat` seam refuse preparation with a typed error and create
//! nothing.

use std::{
    io,
    path::{Path, PathBuf},
    sync::Arc,
};

#[cfg(unix)]
use std::{ffi::OsStr, fs, os::fd::AsRawFd};

use thiserror::Error;

use crate::proc::{OutputSinks, RawChunkSink};
use crate::runner::output::{OUTPUT_CAP_BYTES, PUMP_CHUNK};

use super::output_sink::StoreSink;

#[cfg(unix)]
use super::output_store_unix::{
    NonceGuard, mkdir_private, next_nonce_leaf, open_or_create_private_dir, open_private_dir,
    open_store_file, open_store_root, store_error,
};

/// The per-stream byte cap of the store, single-sourced from the runner's
/// capture cap: store and outcome report against the same bound.
pub const OUTPUT_STORE_CAP_BYTES: usize = OUTPUT_CAP_BYTES;

/// The largest single read response the store serves, one pump-chunk wide.
pub const MAX_OUTPUT_READ_BYTES: usize = PUMP_CHUNK;

/// The one ASCII line appended once a stream hits its cap.
pub const TRUNCATION_MARKER: &str = "[saya: output truncated]\n";

#[cfg(unix)]
const STREAM_DIR: &str = "command-output";

/// Which captured stream a store entry holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputStream {
    Stdout,
    Stderr,
}

impl OutputStream {
    /// The fixed file name the stream is stored under.
    pub fn file_name(self) -> &'static str {
        match self {
            Self::Stdout => "stdout",
            Self::Stderr => "stderr",
        }
    }
}

/// Why command output could not be stored or read: `Debug` and `Display`
/// never leak message text, which could echo stored output or path details.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum CommandOutputError {
    #[error("this command session was not started with an output store")]
    Unavailable,
    #[error("command output store is closed")]
    Closed,
    #[error("output cursor offset {offset} is past the stored bytes")]
    CursorInvalid { offset: u64 },
    #[error("output read exceeds the {max_bytes}-byte limit")]
    ReadTooLarge { max_bytes: usize },
    #[error("command output store failed")]
    Store {
        kind: io::ErrorKind,
        os_code: Option<i32>,
    },
}

/// The typed accounting of one run's output store, available once the run
/// joined and both streams finished; a failure is never reported clean.
#[derive(Debug, Clone)]
pub struct CommandOutputStatus {
    /// Redacted bytes actually written to the store files.
    pub stored_bytes: u64,
    /// Raw child bytes counted dropped after the store sealed.
    pub dropped_bytes: u64,
    /// Every raw byte the child produced, stored or dropped.
    pub raw_total_bytes: u64,
    /// A stream hit its cap: the file is a head prefix ending in the marker.
    pub capped: bool,
    /// The first store failure, `(kind, raw os code)` only — never text.
    pub error: Option<CommandOutputError>,
}

/// The private output store for one command run: a nonce directory holding
/// one append-only file per stream.
pub(crate) struct CommandOutputStore {
    dir: PathBuf,
    streams: [Arc<StoreSink>; 2],
}

impl CommandOutputStore {
    /// Prepares the store under `store_root`:
    /// `<root>/command-output/<nonce>/` with one exclusively created 0600
    /// file per stream, every step descriptor-relative and no-follow. A
    /// store that cannot be prepared refuses the start, and any failure
    /// after the nonce directory exists removes it again, relative to the
    /// retained `command-output` descriptor.
    #[cfg(unix)]
    pub(crate) fn prepare(store_root: &Path) -> Result<Self, CommandOutputError> {
        let root_fd = open_store_root(store_root).map_err(store_error)?;
        let base_fd = open_or_create_private_dir(root_fd.as_raw_fd(), OsStr::new(STREAM_DIR))
            .map_err(store_error)?;
        let leaf = next_nonce_leaf();
        mkdir_private(base_fd.as_raw_fd(), OsStr::new(&leaf)).map_err(store_error)?;
        // Every failure from here on removes the nonce directory again
        // via the retained `command-output` descriptor.
        let guard = NonceGuard::armed(base_fd.as_raw_fd(), &leaf);
        let prepared: Result<Self, CommandOutputError> = (|| {
            let dir_fd =
                open_private_dir(base_fd.as_raw_fd(), OsStr::new(&leaf)).map_err(store_error)?;
            let stdout =
                open_store_file(dir_fd.as_raw_fd(), OutputStream::Stdout).map_err(store_error)?;
            let stderr =
                open_store_file(dir_fd.as_raw_fd(), OutputStream::Stderr).map_err(store_error)?;
            Ok(Self::from_files(
                store_root.join(STREAM_DIR).join(&leaf),
                stdout,
                stderr,
            ))
        })();
        match prepared {
            Ok(store) => {
                guard.defuse();
                Ok(store)
            }
            Err(error) => Err(error),
        }
    }

    /// The output store is Unix-only: platforms without the `openat` seam
    /// refuse preparation with a typed `Unavailable` and create nothing.
    #[cfg(not(unix))]
    pub(crate) fn prepare(_store_root: &Path) -> Result<Self, CommandOutputError> {
        Err(CommandOutputError::Unavailable)
    }

    #[cfg(unix)]
    fn from_files(dir: PathBuf, stdout: fs::File, stderr: fs::File) -> Self {
        Self {
            dir,
            streams: [
                Arc::new(StoreSink::new(stdout)),
                Arc::new(StoreSink::new(stderr)),
            ],
        }
    }

    /// The nonce-named directory holding this run's stored streams.
    pub(crate) fn dir(&self) -> &Path {
        &self.dir
    }

    /// The sink pair the pump seam feeds, one writer thread per stream.
    pub(crate) fn sinks(&self) -> OutputSinks {
        let [stdout, stderr] = &self.streams;
        // Explicit unsizing coercion: `Some(Arc<StoreSink>)` alone does not
        // coerce to `Option<Arc<dyn RawChunkSink>>` through the constructor.
        let stdout: Arc<dyn RawChunkSink> = stdout.clone();
        let stderr: Arc<dyn RawChunkSink> = stderr.clone();
        OutputSinks {
            stdout: Some(stdout),
            stderr: Some(stderr),
        }
    }

    /// The typed store accounting once both streams finished; `None` earlier.
    /// Cross-stream totals saturate at `u64::MAX` rather than wrap, matching
    /// the writer and sink accumulation policy.
    pub(crate) fn output_status(&self) -> Option<CommandOutputStatus> {
        let [stdout, stderr] = &self.streams;
        let stdout = stdout.result()?;
        let stderr = stderr.result()?;
        Some(CommandOutputStatus {
            stored_bytes: stdout.stored_bytes.saturating_add(stderr.stored_bytes),
            dropped_bytes: stdout.dropped_bytes.saturating_add(stderr.dropped_bytes),
            raw_total_bytes: stdout
                .raw_total_bytes
                .saturating_add(stderr.raw_total_bytes),
            capped: stdout.capped || stderr.capped,
            error: stdout
                .error
                .or(stderr.error)
                .map(|(kind, os_code)| CommandOutputError::Store { kind, os_code }),
        })
    }
}
