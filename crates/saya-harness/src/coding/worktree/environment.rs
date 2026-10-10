//! The minimal child command environment for a confined checkout.
//!
//! A confined child command sees **only** the six variables
//! [`ChildEnvironment::apply`] sets — `HOME`, `LANG`, `LC_ALL`, `PATH`,
//! `TMPDIR` and `XDG_CONFIG_HOME` — and nothing else. No provider, database,
//! cloud, SSH or proxy variable the harness process may hold
//! (`OPENAI_API_KEY`, `DATABASE_URL`, `AWS_*`, `SSH_AUTH_SOCK`,
//! `HTTP(S)_PROXY`, …) survives `env_clear`, and [`ChildEnvironment::apply`]
//! is the only place a confined child's environment is ever built: there is
//! no constructor, parameter or accessor that adds a variable or a
//! credential, so the child's secret surface is empty by construction, not
//! by discipline. [`ChildEnvironment::variables`] is the single source of
//! those six variables; [`ChildEnvironment::apply`] is the only place the
//! environment is applied to a command.
//!
//! `HOME`, `TMPDIR` and `XDG_CONFIG_HOME` point at private `0700`
//! directories under `<checkout>/.saya` — the single sandbox root the
//! confinement grants — so the child's home and temp-file use works without
//! reaching the real user tree. The only authorized network shape for the
//! child is none, [`ChildNetworkPolicy::NoEgress`]; `confine_checkout`
//! already refuses any non-empty network allowlist at the confinement
//! boundary. Build output roots are not allocated here: a later packet owns
//! that surface, and until then anything a child leaves under `.saya` is an
//! added change at receipt time.

use std::fmt;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

/// The authorized network shape for a confined child command: none.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChildNetworkPolicy {
    /// The child has no authorized network access of any shape;
    /// `confine_checkout` already refuses any non-empty allowlist.
    NoEgress,
}

/// Failure modes of [`ChildEnvironment::prepare`].
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ChildEnvironmentError {
    /// The program directory is missing, unreadable or not a directory.
    #[error("the program directory is not usable")]
    ProgramDir,
    /// A private child directory could not be prepared as a real, private
    /// directory inside the checkout.
    #[error("a private child directory could not be prepared")]
    PrivateDir,
}

/// The variable names the child may see, in the order [`Debug`] reports
/// them.
const CHILD_VARIABLE_NAMES: [&str; 6] = [
    "HOME",
    "LANG",
    "LC_ALL",
    "PATH",
    "TMPDIR",
    "XDG_CONFIG_HOME",
];

/// The whole environment a confined child command may see.
///
/// [`Debug`] reports the network policy and the variable names only; it
/// never reports a path. The struct holds no credential and no
/// extra-variable surface of any kind.
#[derive(Clone)]
pub struct ChildEnvironment {
    program_dir: PathBuf,
    home: PathBuf,
    tmpdir: PathBuf,
}

impl fmt::Debug for ChildEnvironment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ChildEnvironment")
            .field("network", &self.network())
            .field("variables", &CHILD_VARIABLE_NAMES)
            .finish()
    }
}

impl ChildEnvironment {
    /// Prepares the child environment for `checkout_root`.
    ///
    /// `program_dir` is canonicalised and must be a directory, else
    /// [`ChildEnvironmentError::ProgramDir`]. The private directories
    /// `<checkout_root>/.saya/home` and `<checkout_root>/.saya/tmp` are
    /// created — or, if already present, verified — as real directories,
    /// never symlinks or files, with mode `0700`; each of `.saya`,
    /// `.saya/home` and `.saya/tmp` is re-checked through `symlink_metadata`
    /// before and after creation, else
    /// [`ChildEnvironmentError::PrivateDir`]. On non-Unix hosts preparation
    /// refuses with [`ChildEnvironmentError::PrivateDir`].
    pub fn prepare(
        checkout_root: &Path,
        program_dir: &Path,
    ) -> Result<Self, ChildEnvironmentError> {
        let program_dir =
            std::fs::canonicalize(program_dir).map_err(|_| ChildEnvironmentError::ProgramDir)?;
        if !program_dir.is_dir() {
            return Err(ChildEnvironmentError::ProgramDir);
        }
        let sandbox_root = checkout_root.join(".saya");
        let home = sandbox_root.join("home");
        let tmpdir = sandbox_root.join("tmp");
        ensure_private_dir(&sandbox_root)?;
        ensure_private_dir(&home)?;
        ensure_private_dir(&tmpdir)?;
        Ok(Self {
            program_dir,
            home,
            tmpdir,
        })
    }

    /// The authorized network shape for the child; always
    /// [`ChildNetworkPolicy::NoEgress`].
    pub fn network(&self) -> ChildNetworkPolicy {
        ChildNetworkPolicy::NoEgress
    }

    /// The six variables a confined child command may see, in the order
    /// [`ChildEnvironment::apply`] sets them: `PATH` (the program
    /// directory), `HOME`, `TMPDIR` and `XDG_CONFIG_HOME` (inside the
    /// checkout's private `.saya` tree) and `LC_ALL`, `LANG` pinned to `C`.
    /// This is the only source of the six variables; every path that hands
    /// the environment to a command goes through here.
    pub(crate) fn variables(&self) -> [(&'static str, std::path::PathBuf); 6] {
        [
            ("PATH", self.program_dir.clone()),
            ("HOME", self.home.clone()),
            ("TMPDIR", self.tmpdir.clone()),
            ("XDG_CONFIG_HOME", self.home.clone()),
            ("LC_ALL", PathBuf::from("C")),
            ("LANG", PathBuf::from("C")),
        ]
    }

    /// Applies the child environment to `command`.
    ///
    /// Every inherited variable is cleared first, then exactly the six
    /// allowlisted variables from [`ChildEnvironment::variables`] are set —
    /// the program directory as `PATH`, private directories inside the
    /// checkout's `.saya` tree as `HOME`, `TMPDIR` and `XDG_CONFIG_HOME`,
    /// and `LC_ALL` and `LANG` pinned to `C` for deterministic child
    /// output. Nothing else is set.
    pub fn apply(&self, command: &mut std::process::Command) {
        command.env_clear();
        for (name, value) in self.variables() {
            command.env(name, value);
        }
    }
}

#[cfg(unix)]
fn ensure_private_dir(path: &Path) -> Result<(), ChildEnvironmentError> {
    match std::fs::symlink_metadata(path) {
        Ok(before) if before.file_type().is_dir() => {}
        Ok(_) => return Err(ChildEnvironmentError::PrivateDir),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(ChildEnvironmentError::PrivateDir),
    }
    match std::fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(_) => return Err(ChildEnvironmentError::PrivateDir),
    }
    match std::fs::symlink_metadata(path) {
        Ok(after) if after.file_type().is_dir() => {}
        _ => return Err(ChildEnvironmentError::PrivateDir),
    }
    let mut permissions = std::fs::metadata(path)
        .map_err(|_| ChildEnvironmentError::PrivateDir)?
        .permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(path, permissions).map_err(|_| ChildEnvironmentError::PrivateDir)
}

#[cfg(not(unix))]
fn ensure_private_dir(_path: &Path) -> Result<(), ChildEnvironmentError> {
    Err(ChildEnvironmentError::PrivateDir)
}
