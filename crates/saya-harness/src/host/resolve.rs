//! Program resolution and the built PATH: a bare name resolved against
//! the PATH value the child receives — first regular-file match in order.
//! Not the parent's PATH. There is no staging directory on this lane and no
//! admission battery: both exist to keep exec inside one directory, and
//! this lane has no such directory.

use std::{
    io,
    path::{Path, PathBuf},
    time::Duration,
};

use super::env::{HostEnv, build_env};

/// How long a host call may run when the caller states no narrower bound.
pub const DEFAULT_HOST_TIMEOUT: Duration = Duration::from_secs(600);

/// The executor's typed error surface. Every variant names what the caller
/// must correct — the name shape, the PATH searched, the timeout — never a
/// generic failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum HostError {
    #[error("host command refused: `{program}` is not a bare name — never a path or traversal")]
    NameNotBare { program: String },

    #[error(
        "host command refused: `{program}` is not on the PATH passed to the child; searched: {searched}"
    )]
    NotOnPath { program: String, searched: String },

    #[error("host command refused: the timeout must be at least one second")]
    TimeoutNotPositive,

    #[error(
        "host command refused: the requested timeout {requested}s exceeds the stated ceiling {ceiling}s"
    )]
    TimeoutExceedsCeiling { requested: u64, ceiling: u64 },

    #[error(
        "host command refused: environment variable `{name}` is not a well-formed NAME=value name"
    )]
    EnvNameInvalid { name: String },

    #[error("host command failed: the child could not be started: {source}")]
    Spawn { source: io::Error },

    #[error("host command failed: the child's exit could not be observed: {source}")]
    WaitFailed { source: io::Error },
}

/// The host executor's configuration: the PATH the child receives, the
/// timeout ceiling a call may narrow but never widen, and the explicitly
/// passed environment. Built once, shared across calls.
#[derive(Debug, Clone)]
pub struct HostConfig {
    path: String,
    timeout: Duration,
    env: HostEnv,
}

impl HostConfig {
    /// Builds the config. `path` is the exact PATH value the child
    /// receives — resolution searches it, in order. The timeout is the
    /// ceiling; zero is refused.
    pub fn new(path: impl Into<String>, timeout: Duration) -> Result<Self, HostError> {
        let timeout_secs = timeout.as_secs();
        if timeout_secs == 0 {
            return Err(HostError::TimeoutNotPositive);
        }
        Ok(Self {
            path: path.into(),
            timeout,
            env: HostEnv::default(),
        })
    }

    /// Attaches the caller's explicitly passed variables. Later entries
    /// with the same name win; the base PATH, HOME, and TMPDIR values are
    /// set separately (see [`HostEnv`]).
    pub fn with_extra_env(
        mut self,
        vars: impl IntoIterator<Item = (String, String)>,
    ) -> Result<Self, HostError> {
        for (name, value) in vars {
            self.env.insert(name, value)?;
        }
        Ok(self)
    }

    /// The ceiling a call may narrow but never widen.
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// Narrows the ceiling for one call. Zero is refused; widening is
    /// refused with both numbers.
    pub fn narrow_timeout(&self, requested_seconds: u64) -> Result<Duration, HostError> {
        if requested_seconds == 0 {
            return Err(HostError::TimeoutNotPositive);
        }
        let narrowed = Duration::from_secs(requested_seconds);
        if narrowed > self.timeout {
            return Err(HostError::TimeoutExceedsCeiling {
                requested: requested_seconds,
                ceiling: self.timeout.as_secs(),
            });
        }
        Ok(narrowed)
    }

    /// Resolves a bare name against the PATH the child receives: the first
    /// regular-file match in order. A directory or any other non-file entry
    /// is skipped, not exec'd. Path-shaped and traversal names refuse here
    /// too, so resolution never depends on the caller having constructed a
    /// [`super::HostCommand`] first.
    pub fn resolve(&self, program: &str) -> Result<PathBuf, HostError> {
        if !saya_types::is_bare_name(program) {
            return Err(HostError::NameNotBare {
                program: program.to_owned(),
            });
        }
        for dir in self.path.split(':') {
            if dir.is_empty() {
                continue;
            }
            let candidate = Path::new(dir).join(program);
            if is_regular_file(&candidate) {
                return Ok(candidate);
            }
        }
        Err(HostError::NotOnPath {
            program: program.to_owned(),
            searched: self.path.clone(),
        })
    }

    /// Builds the child's environment onto the command: `env_clear`, then
    /// the base PATH, HOME, and TMPDIR values plus the caller's explicit
    /// variables. Nothing else from the parent reaches the child.
    pub fn apply_env(&self, command: &mut std::process::Command) {
        command.env_clear();
        for (name, value) in build_env(&self.path, &self.env) {
            command.env(name, value);
        }
    }
}

/// True when the path names a regular file. Symlinks are followed: a link
/// to a regular file matches its target, in resolution order; anything
/// else — directory, fifo, device, missing — is not a match.
fn is_regular_file(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|meta| meta.is_file())
}
