#[cfg(unix)]
use std::fs::{self, OpenOptions};
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _};
#[cfg(unix)]
use std::path::Path;
use std::path::PathBuf;

#[cfg(unix)]
use super::command::GitError;
use super::compatibility::GitCompatibility;

pub(super) struct GitEnvironment {
    pub(super) root: PathBuf,
    pub(super) worktrees: PathBuf,
    pub(super) hooks: PathBuf,
    pub(super) temporary: PathBuf,
    pub(super) empty_global_config: PathBuf,
    pub(super) executable: PathBuf,
    compatibility: std::sync::OnceLock<GitCompatibility>,
}

impl GitEnvironment {
    #[cfg(unix)]
    pub(super) fn new(executable: PathBuf, state_root: &Path) -> Result<Self, GitError> {
        if !state_root.is_absolute() {
            return Err(GitError::InvalidStateRoot);
        }
        let input_metadata =
            fs::symlink_metadata(state_root).map_err(|_| GitError::InvalidStateRoot)?;
        if input_metadata.file_type().is_symlink() {
            return Err(GitError::InvalidStateRoot);
        }
        let root = fs::canonicalize(state_root).map_err(|_| GitError::InvalidStateRoot)?;
        let metadata = fs::symlink_metadata(&root).map_err(|_| GitError::InvalidStateRoot)?;
        if !metadata.is_dir()
            || metadata.file_type().is_symlink()
            || !private_mode(&metadata)
            || root.to_str().is_none()
        {
            return Err(GitError::InvalidStateRoot);
        }
        let worktrees = root.join("worktrees");
        let hooks = root.join("empty-hooks");
        let temporary = root.join("tmp");
        let empty_global_config = root.join("empty.gitconfig");
        ensure_empty_dir(&hooks)?;
        ensure_private_dir(&worktrees)?;
        ensure_private_dir(&temporary)?;
        ensure_empty_file(&empty_global_config)?;
        Ok(Self {
            root,
            worktrees,
            hooks,
            temporary,
            empty_global_config,
            executable,
            compatibility: std::sync::OnceLock::new(),
        })
    }

    /// The detected compatibility, or the conservative minimum until the
    /// first bounded `git --version` answers.
    pub(super) fn compatibility(&self) -> GitCompatibility {
        self.compatibility
            .get()
            .copied()
            .unwrap_or_else(GitCompatibility::assumed_minimum)
    }

    pub(super) fn cached_compatibility(&self) -> Option<GitCompatibility> {
        self.compatibility.get().copied()
    }

    pub(super) fn set_compatibility(&self, compatibility: GitCompatibility) {
        let _ = self.compatibility.set(compatibility);
    }

    pub(super) fn apply(&self, command: &mut std::process::Command) {
        command
            .env_clear()
            .env("HOME", &self.root)
            .env(
                "PATH",
                self.executable.parent().expect("validated executable"),
            )
            .env("TMPDIR", &self.temporary)
            .env("XDG_CONFIG_HOME", &self.root)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", &self.empty_global_config)
            .env("GIT_ATTR_NOSYSTEM", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_ASKPASS", "")
            .env("SSH_ASKPASS", "")
            .env("GIT_PAGER", "cat")
            .env("PAGER", "cat")
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("GIT_NO_REPLACE_OBJECTS", "1")
            .env("LC_ALL", "C")
            .env("LANG", "C");
        for (key, value) in self.config_overrides() {
            command.arg("-c").arg(format!("{key}={value}"));
        }
    }

    fn config_overrides(&self) -> [(&'static str, String); 10] {
        [
            ("core.hooksPath", self.hooks.to_string_lossy().into_owned()),
            ("core.fsmonitor", "false".to_owned()),
            ("core.untrackedCache", "false".to_owned()),
            ("core.attributesFile", String::new()),
            ("core.pager", "cat".to_owned()),
            ("diff.external", String::new()),
            ("credential.helper", String::new()),
            ("submodule.recurse", "false".to_owned()),
            ("gc.auto", "0".to_owned()),
            ("maintenance.auto", "false".to_owned()),
        ]
    }
}

#[cfg(unix)]
fn ensure_private_dir(path: &Path) -> Result<(), GitError> {
    match create_private_dir(path) {
        Ok(()) => set_private_mode(path)?,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(_) => return Err(GitError::InvalidStateRoot),
    }
    let metadata = fs::symlink_metadata(path).map_err(|_| GitError::InvalidStateRoot)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() || !private_mode(&metadata) {
        return Err(GitError::InvalidStateRoot);
    }
    Ok(())
}

#[cfg(unix)]
fn ensure_empty_dir(path: &Path) -> Result<(), GitError> {
    ensure_private_dir(path)?;
    let mut entries = fs::read_dir(path).map_err(|_| GitError::InvalidStateRoot)?;
    match entries.next() {
        None => Ok(()),
        Some(Ok(_)) | Some(Err(_)) => Err(GitError::InvalidStateRoot),
    }
}

#[cfg(unix)]
fn ensure_empty_file(path: &Path) -> Result<(), GitError> {
    match create_private_file(path) {
        Ok(_) => set_private_file_mode(path)?,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(_) => return Err(GitError::InvalidStateRoot),
    }
    let metadata = fs::symlink_metadata(path).map_err(|_| GitError::InvalidStateRoot)?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() != 0
        || !private_mode(&metadata)
    {
        return Err(GitError::InvalidStateRoot);
    }
    Ok(())
}

#[cfg(unix)]
fn create_private_dir(path: &Path) -> std::io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.mode(0o700).create(path)
}

#[cfg(unix)]
fn create_private_file(path: &Path) -> std::io::Result<fs::File> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

#[cfg(unix)]
fn private_mode(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt as _;
    metadata.mode() & 0o077 == 0 && metadata.uid() == unsafe { libc::geteuid() }
}

#[cfg(unix)]
fn set_private_mode(path: &Path) -> Result<(), GitError> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|_| GitError::InvalidStateRoot)
}

#[cfg(unix)]
fn set_private_file_mode(path: &Path) -> Result<(), GitError> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|_| GitError::InvalidStateRoot)
}
