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
//! reaching the real user tree. Helpers share one working checkout, so
//! each helper's scratch tree is its own: [`ScratchScope::Helper`] puts them
//! under `<checkout>/.saya/helper-<agent-id>/{home,tmp}` and the root keeps
//! `<checkout>/.saya/{home,tmp}`. `.saya` is the checkout's authorized output
//! root and stays out of the candidate, so no scratch file reaches the
//! operator's review.
//!
//! The only authorized network shape for the child is none,
//! [`ChildNetworkPolicy::NoEgress`]; `confine_checkout` already refuses any
//! non-empty network allowlist at the confinement boundary.

use std::fmt;
use std::path::{Path, PathBuf};

use saya_types::AgentId;

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

/// Which tree under the sandbox root holds the scratch layout.
///
/// A managed checkout is app-owned, so its scratch sits directly in
/// `<checkout>/.saya`. An in-place root is the user's folder: its scratch is
/// one level deeper, in `<folder>/.saya/inplace`, so the project's own
/// `.saya/config.toml` is never part of the scratch layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScratchBase {
    /// `<root>/.saya` for an app-owned managed checkout.
    Checkout,
    /// `<root>/.saya/inplace` for the user's own folder.
    InPlace,
}

impl ScratchBase {
    fn directory(self) -> Option<&'static str> {
        match self {
            Self::Checkout => None,
            Self::InPlace => Some("inplace"),
        }
    }
}

/// Which scratch tree a confined command's `HOME`, `TMPDIR` and
/// `XDG_CONFIG_HOME` live in.
///
/// Both are private `0700` directories inside the checkout's `.saya` tree; the
/// scope only decides whose they are, so two helpers writing the same working
/// checkout never share a home or a temp directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScratchScope<'a> {
    /// The root's own commands: `<base>/{home,tmp}`.
    Root,
    /// One helper's commands: `<base>/helper-<agent-id>/{home,tmp}`.
    Helper(&'a AgentId),
}

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
    /// Prepares the child environment for `checkout_root` under `base` and
    /// `scope`.
    ///
    /// `program_dir` is canonicalised and must be a directory, else
    /// [`ChildEnvironmentError::ProgramDir`]. The private directories the
    /// base and scope name — `<checkout_root>/.saya/{home,tmp}` (or
    /// `<checkout_root>/.saya/inplace/...` for an in-place root) for
    /// [`ScratchScope::Root`], and the same base with
    /// `helper-<agent-id>/{home,tmp}` for [`ScratchScope::Helper`] — are
    /// created, or, if already present, verified, as real directories, never
    /// symlinks or files, with mode `0700`; every directory on the way
    /// (`.saya`, the in-place level, the helper directory when there is one,
    /// and the two leaves) is re-checked through `symlink_metadata` before and
    /// after creation, else [`ChildEnvironmentError::PrivateDir`]. On non-Unix
    /// hosts preparation refuses with [`ChildEnvironmentError::PrivateDir`].
    pub fn prepare(
        checkout_root: &Path,
        program_dir: &Path,
        base: ScratchBase,
        scope: ScratchScope<'_>,
    ) -> Result<Self, ChildEnvironmentError> {
        let program_dir =
            std::fs::canonicalize(program_dir).map_err(|_| ChildEnvironmentError::ProgramDir)?;
        if !program_dir.is_dir() {
            return Err(ChildEnvironmentError::ProgramDir);
        }
        let saya_root = checkout_root.join(".saya");
        ensure_private_dir(&saya_root)?;
        let sandbox_root = match base.directory() {
            Some(directory) => {
                let in_place = saya_root.join(directory);
                ensure_private_dir(&in_place)?;
                in_place
            }
            None => saya_root,
        };
        let scratch_root = match scope {
            ScratchScope::Root => sandbox_root,
            ScratchScope::Helper(agent) => {
                let helper_root = sandbox_root.join(format!("helper-{}", agent.as_str()));
                ensure_private_dir(&helper_root)?;
                helper_root
            }
        };
        let home = scratch_root.join("home");
        let tmpdir = scratch_root.join("tmp");
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

    /// The private directory this scope's `HOME` points at: the root's own
    /// `<checkout>/.saya/home`, or one helper's under
    /// `<checkout>/.saya/helper-<agent-id>/home`.
    pub fn home(&self) -> &Path {
        &self.home
    }

    /// The private directory this scope's `TMPDIR` points at, beside
    /// [`home`](Self::home).
    pub fn tmpdir(&self) -> &Path {
        &self.tmpdir
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

/// Removes an in-place root's scratch tree at job end, but only when it
/// holds the known layout: `home`, `tmp`, and `helper-<agent>/{home,tmp}`
/// directories. Any other entry, a symlink, or an unreadable entry leaves the
/// whole tree untouched — a user's `.saya/inplace` is never guessed at. The
/// parent `.saya` is removed too when the scratch was its only content; an
/// empty-directory removal cannot touch a user file, and any other entry
/// leaves it in place.
pub fn remove_in_place_scratch(root: &Path) -> Result<(), ChildEnvironmentError> {
    #[cfg(unix)]
    {
        let saya_root = root.join(".saya");
        let in_place = saya_root.join("inplace");
        match std::fs::symlink_metadata(&in_place) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Ok(metadata) if metadata.file_type().is_dir() => {}
            _ => return Err(ChildEnvironmentError::PrivateDir),
        }
        validate_scratch_layout(&in_place)?;
        std::fs::remove_dir_all(&in_place).map_err(|_| ChildEnvironmentError::PrivateDir)?;
        // The job created `.saya` only to hold its scratch; remove it too when
        // it is empty. `remove_dir` refuses a non-empty directory, so a user
        // file there is never touched, and a failed removal is left as it is.
        let _ = std::fs::remove_dir(&saya_root);
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = root;
        Err(ChildEnvironmentError::PrivateDir)
    }
}

#[cfg(unix)]
fn validate_scratch_layout(dir: &Path) -> Result<(), ChildEnvironmentError> {
    for entry in std::fs::read_dir(dir).map_err(|_| ChildEnvironmentError::PrivateDir)? {
        let entry = entry.map_err(|_| ChildEnvironmentError::PrivateDir)?;
        let name = entry
            .file_name()
            .to_str()
            .ok_or(ChildEnvironmentError::PrivateDir)?
            .to_owned();
        let metadata = std::fs::symlink_metadata(entry.path())
            .map_err(|_| ChildEnvironmentError::PrivateDir)?;
        if !metadata.file_type().is_dir() {
            return Err(ChildEnvironmentError::PrivateDir);
        }
        if name == "home" || name == "tmp" {
            continue;
        }
        let Some(agent) = name.strip_prefix("helper-") else {
            return Err(ChildEnvironmentError::PrivateDir);
        };
        if agent.is_empty()
            || !agent
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        {
            return Err(ChildEnvironmentError::PrivateDir);
        }
        validate_helper_layout(&entry.path())?;
    }
    Ok(())
}

#[cfg(unix)]
fn validate_helper_layout(dir: &Path) -> Result<(), ChildEnvironmentError> {
    for entry in std::fs::read_dir(dir).map_err(|_| ChildEnvironmentError::PrivateDir)? {
        let entry = entry.map_err(|_| ChildEnvironmentError::PrivateDir)?;
        let name = entry
            .file_name()
            .to_str()
            .ok_or(ChildEnvironmentError::PrivateDir)?
            .to_owned();
        let metadata = std::fs::symlink_metadata(entry.path())
            .map_err(|_| ChildEnvironmentError::PrivateDir)?;
        if !metadata.file_type().is_dir() || !matches!(name.as_str(), "home" | "tmp") {
            return Err(ChildEnvironmentError::PrivateDir);
        }
    }
    Ok(())
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
