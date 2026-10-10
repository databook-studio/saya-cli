#![cfg(any(target_os = "macos", target_os = "linux"))]

//! The materialization target contract: an empty private directory, or —
//! for `git worktree add --no-checkout` — exactly one entry, a regular
//! `.git` pointer file. The writer never creates, modifies, or rolls back
//! `.git`; rollback removes only the entries this run created.

use std::{
    collections::HashMap,
    ffi::{CString, OsStr},
    fs, io,
    os::{
        fd::{AsRawFd, RawFd},
        unix::{
            ffi::OsStrExt,
            fs::{MetadataExt, PermissionsExt},
        },
    },
};

use super::{
    SnapshotError,
    capture_dest::capture_io,
    capture_dest_private::{mkdir_private_at, owned_private, privatize_dir},
    types::{SnapshotFileMode, SnapshotPath},
};
use crate::workspace::{
    Workspace,
    fd::{DirStream, open_dir_at, stat_at},
};

pub(super) const MAX_ROOT_ENTRIES: usize = 4096;

pub(super) struct TargetWriter<'a> {
    workspace: &'a Workspace,
    names: HashMap<String, (String, bool)>,
    created_dirs: Vec<String>,
    created_files: Vec<String>,
}

impl<'a> TargetWriter<'a> {
    pub(super) fn open(workspace: &'a Workspace) -> Result<Self, SnapshotError> {
        let root_fd = workspace
            .root_dir_fd()
            .try_clone()
            .map_err(|error| capture_io("open materialization target", &error))?;
        let root = fs::File::from(root_fd);
        let metadata = root
            .metadata()
            .map_err(|error| capture_io("verify materialization target", &error))?;
        if !metadata.is_dir() || !owned_private(&metadata) {
            return Err(SnapshotError::MaterializeTargetInvalid);
        }
        ensure_empty(&root)?;
        Ok(Self {
            workspace,
            names: HashMap::new(),
            created_dirs: Vec::new(),
            created_files: Vec::new(),
        })
    }

    pub(super) fn create_file(
        &mut self,
        path: &SnapshotPath,
        _mode: SnapshotFileMode,
    ) -> Result<fs::File, SnapshotError> {
        let path = path.as_str();
        let components: Vec<&str> = path.split('/').collect();
        // Defence in depth: SnapshotPath::parse already refuses `.git` components.
        if components[0].eq_ignore_ascii_case(".git") {
            return Err(SnapshotError::UnsafePath);
        }
        for depth in 0..components.len() - 1 {
            let prefix = components[..=depth].join("/");
            self.require_created_dir(&prefix)?;
        }
        if self.names.contains_key(&path.to_ascii_lowercase()) {
            return Err(SnapshotError::UnsafePath);
        }
        for depth in 0..components.len() - 1 {
            let prefix = components[..=depth].join("/");
            if !self.created_dirs.iter().any(|existing| existing == &prefix) {
                self.create_dir(&prefix)?;
            }
        }
        let anchor = self
            .workspace
            .anchor(path, false)
            .map_err(SnapshotError::from_workspace_error)?;
        let file = anchor
            .open_file(libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL, 0o600)
            .map_err(|error| capture_io("create materialized file", &error))?;
        self.names
            .insert(path.to_ascii_lowercase(), (path.to_string(), false));
        self.created_files.push(path.to_string());
        Ok(file)
    }

    fn require_created_dir(&self, prefix: &str) -> Result<(), SnapshotError> {
        match self.names.get(&prefix.to_ascii_lowercase()) {
            Some((actual, is_dir)) if actual.as_str() == prefix && *is_dir => Ok(()),
            Some(_) => Err(SnapshotError::UnsafePath),
            None => Ok(()),
        }
    }

    fn create_dir(&mut self, prefix: &str) -> Result<(), SnapshotError> {
        let anchor = self
            .workspace
            .anchor(prefix, false)
            .map_err(SnapshotError::from_workspace_error)?;
        if let Err(error) = mkdir_private_at(anchor.dir_fd(), anchor.name()) {
            if error.kind() == io::ErrorKind::AlreadyExists {
                return Err(SnapshotError::UnsafePath);
            }
            return Err(capture_io("create materialized directory", &error));
        }
        self.created_dirs.push(prefix.to_string());
        let dir = fs::File::from(
            open_dir_at(anchor.dir_fd(), anchor.name())
                .map_err(|error| capture_io("reopen materialized directory", &error))?,
        );
        let opened = dir
            .metadata()
            .map_err(|error| capture_io("reopen materialized directory", &error))?;
        let named = anchor
            .stat_final()
            .map_err(|error| capture_io("reopen materialized directory", &error))?;
        if !opened.is_dir() || !named.is_dir() || (opened.dev(), opened.ino()) != named.identity() {
            return Err(SnapshotError::CaptureIo {
                description: "keep a created materialized directory under its name".to_string(),
                source: None,
            });
        }
        privatize_dir(&dir, "materialized directory")?;
        self.names
            .insert(prefix.to_ascii_lowercase(), (prefix.to_string(), true));
        Ok(())
    }

    pub(super) fn finish_file(
        &mut self,
        file: fs::File,
        mode: SnapshotFileMode,
    ) -> Result<(), SnapshotError> {
        let bits = match mode {
            SnapshotFileMode::Regular => 0o644,
            SnapshotFileMode::Executable => 0o755,
        };
        file.set_permissions(fs::Permissions::from_mode(bits))
            .map_err(|error| capture_io("set materialized file mode", &error))?;
        file.sync_all()
            .map_err(|error| capture_io("sync materialized file", &error))?;
        Ok(())
    }

    /// Removes only the entries this run created, so a pre-existing `.git`
    /// pointer file survives.
    pub(super) fn rollback(&mut self) -> Result<(), SnapshotError> {
        let mut failures = 0usize;
        for path in self.created_files.iter().rev() {
            match self.workspace.anchor(path, false) {
                Ok(anchor) => {
                    if anchor.unlink_final().is_err() {
                        failures += 1;
                    }
                }
                Err(_) => failures += 1,
            }
        }
        for dir in self.created_dirs.iter().rev() {
            match self.workspace.anchor(dir, false) {
                Ok(anchor) => {
                    if rmdir_at(anchor.dir_fd(), anchor.name()).is_err() {
                        failures += 1;
                    }
                }
                Err(_) => failures += 1,
            }
        }
        self.created_files.clear();
        self.created_dirs.clear();
        self.names.clear();
        if failures == 0 {
            Ok(())
        } else {
            Err(SnapshotError::MaterializeCleanupFailed)
        }
    }

    pub(super) fn sync_all(&mut self) -> Result<(), SnapshotError> {
        for dir in self.created_dirs.iter().rev() {
            let anchor = self
                .workspace
                .anchor(dir, false)
                .map_err(SnapshotError::from_workspace_error)?;
            let synced = fs::File::from(
                open_dir_at(anchor.dir_fd(), anchor.name())
                    .map_err(|error| capture_io("sync materialized directory", &error))?,
            );
            synced
                .sync_all()
                .map_err(|error| capture_io("sync materialized directory", &error))?;
        }
        let root = fs::File::from(
            self.workspace
                .root_dir_fd()
                .try_clone()
                .map_err(|error| capture_io("sync materialization target", &error))?,
        );
        root.sync_all()
            .map_err(|error| capture_io("sync materialization target", &error))
    }
}

/// Refuses every shape except empty, or exactly one regular-file `.git`
/// entry — git's own `--no-checkout` worktree pointer, never content.
fn ensure_empty(root: &fs::File) -> Result<(), SnapshotError> {
    let mut stream = DirStream::open_at(root.as_raw_fd(), OsStr::new("."))
        .map_err(|error| capture_io("list materialization target", &error))?;
    for _ in 0..MAX_ROOT_ENTRIES {
        let next = stream
            .next()
            .map_err(|error| capture_io("list materialization target", &error));
        match next? {
            None => return Ok(()),
            Some((name, _)) if name.as_bytes() == b"." || name.as_bytes() == b".." => continue,
            Some((name, _)) if name.as_bytes() == b".git" => {
                let final_stat = stat_at(root.as_raw_fd(), &name)
                    .map_err(|_| SnapshotError::MaterializeTargetInvalid)?;
                if !final_stat.is_file() {
                    return Err(SnapshotError::MaterializeTargetInvalid);
                }
            }
            Some(_) => return Err(SnapshotError::MaterializeTargetInvalid),
        }
    }
    Err(SnapshotError::MaterializeTargetInvalid)
}

fn rmdir_at(dir_fd: RawFd, name: &OsStr) -> io::Result<()> {
    let c = CString::new(name.as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "NUL in component"))?;
    let rc = unsafe { libc::unlinkat(dir_fd, c.as_ptr(), libc::AT_REMOVEDIR) };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
