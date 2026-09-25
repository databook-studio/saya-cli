//! Opening the run's scratch database (ADR 0003): one DuckDB file per run at
//! `runs/<id>/scratch.duckdb`, opened via the raw `duckdb` crate with the
//! security configuration pinned by `tests/scratch_semantics.rs` — external
//! access **off** (the 2026-09-11 sign-off), extension autoload and community
//! extensions off, no persistent secrets, configuration locked. Not a
//! `DatabaseConnector` and never a registry entry: no type-level path may run
//! from a scratch write to a user database.

use std::{
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

#[cfg(unix)]
use std::fs;

use duckdb::{AccessMode, Config, Connection, InterruptHandle};

use super::ScratchError;

/// The scratch file's name inside the run directory.
pub const SCRATCH_FILE_NAME: &str = "scratch.duckdb";

/// The per-statement ceiling. The run engine may narrow it per step with
/// [`ScratchDb::with_query_timeout`]; it never widens past the run's budgets.
pub const SCRATCH_QUERY_TIMEOUT: Duration = Duration::from_secs(30);

/// The opened scratch database. One DuckDB file per run, addressed only by
/// the run's tools, dying with the run directory — nothing here outlives it.
#[derive(Clone)]
pub struct ScratchDb {
    pub(super) connection: Arc<Mutex<Connection>>,
    pub(super) interrupt: Arc<InterruptHandle>,
    pub(super) query_timeout: Duration,
}

impl ScratchDb {
    /// Opens — creating if missing — the run's scratch file under the pinned
    /// configuration. The configuration is the same engine hardening the
    /// DuckDB connector applies, with external access off: the M4-1 pinning
    /// measured that with it on, `INSTALL httpfs` reaches the network — egress
    /// the fetch policy never sees — and that with it off every route out
    /// (install, load, http and local file reads) is refused at the permission
    /// layer. The flag is all-or-nothing, so there is no file-reading
    /// carve-out at all; corpus data arrives through the workspace tools.
    pub fn open(run_root: &Path) -> Result<Self, ScratchError> {
        let path = run_root.join(SCRATCH_FILE_NAME);
        let connection =
            Connection::open_with_flags(&path, scratch_config()?).map_err(|error| {
                ScratchError::Open {
                    context: error.to_string(),
                }
            })?;
        harden_file_mode(&path)?;
        let interrupt = connection.interrupt_handle();
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
            interrupt,
            query_timeout: SCRATCH_QUERY_TIMEOUT,
        })
    }

    /// Narrows the per-statement timeout, for the run engine to follow a
    /// step's remaining budget. Never called with a longer value.
    pub fn with_query_timeout(mut self, query_timeout: Duration) -> Self {
        self.query_timeout = query_timeout;
        self
    }
}

/// The scratch configuration, verbatim from the merged pinning test
/// (`tests/scratch_semantics.rs`, fallback block) and locked at open:
/// `lock_configuration` refuses `SET enable_external_access` in both
/// directions afterwards.
fn scratch_config() -> Result<Config, ScratchError> {
    Config::default()
        .access_mode(AccessMode::ReadWrite)
        .and_then(|item| item.enable_external_access(false))
        .and_then(|item| item.enable_autoload_extension(false))
        .and_then(|item| item.with("allow_community_extensions", "false"))
        .and_then(|item| item.with("allow_persistent_secrets", "false"))
        .and_then(|item| item.with("lock_configuration", "true"))
        .map_err(|_| ScratchError::Open {
            context: "scratch security configuration failed to build".to_string(),
        })
}

/// DuckDB creates the database file at 0644 (pinned by M4-1) — group- and
/// world-readable, carrying no protection of its own. The run directory is
/// 0700, but the engine does not rely on the directory alone: the file is
/// restricted to 0600 here, on every open (fresh create *and* resume, so a
/// loose mode cannot survive a resume).
#[cfg(unix)]
fn harden_file_mode(path: &Path) -> Result<(), ScratchError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|error| {
        ScratchError::FileMode {
            path: path.to_path_buf(),
            source: error,
        }
    })
}

#[cfg(not(unix))]
#[inline]
fn harden_file_mode(_path: &Path) -> Result<(), ScratchError> {
    Ok(())
}
