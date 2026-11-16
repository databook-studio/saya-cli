//! The Windows materialization target contract: an empty directory, or —
//! for `git worktree add --no-checkout` — exactly one entry, a regular
//! `.git` pointer file. Paths go through `Workspace::target`, so every
//! component is re-checked for containment and reparse points, and the
//! writer never creates, modifies, or rolls back `.git`. Windows has no
//! POSIX ownership or directory fsync: the target is accepted when it is a
//! real directory that is not a reparse point, each file is flushed as it is
//! written, and rollback removes only the entries this run created.

use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    io,
    os::windows::fs::{OpenOptionsExt as _, symlink_dir, symlink_file},
    path::{Path, PathBuf},
};

use super::{
    SnapshotError,
    capture_dest::capture_io,
    types::{SnapshotFileMode, SnapshotPath},
};
use crate::workspace::{Workspace, contain::FILE_FLAG_OPEN_REPARSE_POINT, windows_identity};

pub(super) const MAX_ROOT_ENTRIES: usize = 4096;

pub(super) struct TargetWriter<'a> {
    workspace: &'a Workspace,
    names: HashMap<String, (String, bool)>,
    created_dirs: Vec<String>,
    created_files: Vec<String>,
}

impl<'a> TargetWriter<'a> {
    pub(super) fn open(workspace: &'a Workspace) -> Result<Self, SnapshotError> {
        let root = workspace.root();
        let metadata = fs::symlink_metadata(root)
            .map_err(|error| capture_io("verify materialization target", &error))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(SnapshotError::MaterializeTargetInvalid);
        }
        if windows_identity::path_is_reparse_point(root)
            .map_err(|error| capture_io("verify materialization target", &error))?
        {
            return Err(SnapshotError::MaterializeTargetInvalid);
        }
        ensure_empty(root)?;
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
        let full = self.prepare_target(path)?;
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(&full)
            .map_err(|error| capture_io("create materialized file", &error))?;
        self.record_created_file(path);
        Ok(file)
    }

    /// Creates the target as a symlink whose target bytes were already
    /// verified against the stored object. Windows needs the link kind up
    /// front: a target that resolves to an existing directory becomes a
    /// directory link, everything else (including a dangling target) a file
    /// link.
    pub(super) fn create_symlink(
        &mut self,
        path: &SnapshotPath,
        target: &[u8],
    ) -> Result<(), SnapshotError> {
        let full = self.prepare_target(path)?;
        let text = std::str::from_utf8(target).map_err(|_| SnapshotError::unsafe_path())?;
        let parent = full.parent().unwrap_or(self.workspace.root());
        let resolved = parent.join(text.replace('/', "\\"));
        let is_dir = fs::metadata(&resolved)
            .map(|metadata| metadata.is_dir())
            .unwrap_or(false);
        let created = if is_dir {
            symlink_dir(text, &full)
        } else {
            symlink_file(text, &full)
        };
        created.map_err(|error| capture_io("create materialized symlink", &error))?;
        self.record_created_file(path);
        Ok(())
    }

    /// Refuses `.git`, creates the parent directories in order, refuses a
    /// name collision (case-insensitively), and returns the validated final
    /// path, which must still be absent.
    fn prepare_target(&mut self, path: &SnapshotPath) -> Result<PathBuf, SnapshotError> {
        let path = path.as_str();
        let components: Vec<&str> = path.split('/').collect();
        // Defence in depth: SnapshotPath::parse already refuses `.git` components.
        if components[0].eq_ignore_ascii_case(".git") {
            return Err(SnapshotError::unsafe_path());
        }
        for depth in 0..components.len() - 1 {
            let prefix = components[..=depth].join("/");
            self.require_created_dir(&prefix)?;
        }
        if self.names.contains_key(&path.to_ascii_lowercase()) {
            return Err(SnapshotError::unsafe_path());
        }
        for depth in 0..components.len() - 1 {
            let prefix = components[..=depth].join("/");
            if !self.created_dirs.iter().any(|existing| existing == &prefix) {
                self.create_dir(&prefix)?;
            }
        }
        let full = self
            .workspace
            .target(path, false)
            .map_err(SnapshotError::from_workspace_error)?;
        match fs::symlink_metadata(&full) {
            Ok(_) => Err(SnapshotError::unsafe_path()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(full),
            Err(error) => Err(capture_io("scan materialized file", &error)),
        }
    }

    fn record_created_file(&mut self, path: &SnapshotPath) {
        let path = path.as_str();
        self.names
            .insert(path.to_ascii_lowercase(), (path.to_string(), false));
        self.created_files.push(path.to_string());
    }

    fn require_created_dir(&self, prefix: &str) -> Result<(), SnapshotError> {
        match self.names.get(&prefix.to_ascii_lowercase()) {
            Some((actual, is_dir)) if actual.as_str() == prefix && *is_dir => Ok(()),
            Some(_) => Err(SnapshotError::unsafe_path()),
            None => Ok(()),
        }
    }

    fn create_dir(&mut self, prefix: &str) -> Result<(), SnapshotError> {
        // `target` answers with the path whether or not the final component
        // exists, so ask the filesystem. An existing directory is not this
        // run's to remove. A missing one is created here and recorded for
        // rollback; a creation race that already made the name refuses,
        // exactly as the unix O_EXCL mkdir does.
        let full = self
            .workspace
            .target(prefix, false)
            .map_err(SnapshotError::from_workspace_error)?;
        match fs::symlink_metadata(&full) {
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => return Err(SnapshotError::unsafe_path()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let created = self
                    .workspace
                    .target(prefix, true)
                    .map_err(SnapshotError::from_workspace_error)?;
                fs::create_dir(&created)
                    .map_err(|error| capture_io("create materialized directory", &error))?;
                self.created_dirs.push(prefix.to_string());
            }
            Err(error) => return Err(capture_io("scan materialized directory", &error)),
        }
        if windows_identity::path_is_reparse_point(&full)
            .map_err(|error| capture_io("verify materialized directory", &error))?
        {
            return Err(SnapshotError::unsafe_path());
        }
        self.names
            .insert(prefix.to_ascii_lowercase(), (prefix.to_string(), true));
        Ok(())
    }

    pub(super) fn finish_file(
        &mut self,
        file: fs::File,
        mode: SnapshotFileMode,
    ) -> Result<(), SnapshotError> {
        if mode == SnapshotFileMode::Symlink {
            return Err(SnapshotError::UnsupportedFileType);
        }
        // Windows has no execute bit; both snapshot modes materialize as an
        // ordinary writable file. Flush before the file is accepted.
        file.sync_all()
            .map_err(|error| capture_io("sync materialized file", &error))?;
        Ok(())
    }

    /// Removes only the entries this run created, so a pre-existing `.git`
    /// pointer file survives. Every removal re-runs the containment walk and
    /// refuses to remove a reparse point.
    pub(super) fn rollback(&mut self) -> Result<(), SnapshotError> {
        let mut failures = 0usize;
        for path in self.created_files.iter().rev() {
            match self.workspace.target(path, false) {
                Ok(full) if removable_entry(&full) => {
                    if fs::remove_file(&full).is_err() {
                        failures += 1;
                    }
                }
                _ => failures += 1,
            }
        }
        for dir in self.created_dirs.iter().rev() {
            match self.workspace.target(dir, false) {
                Ok(full) if removable_entry(&full) => {
                    if fs::remove_dir(&full).is_err() {
                        failures += 1;
                    }
                }
                _ => failures += 1,
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

    /// Files were flushed as they were written; Windows has no directory
    /// fsync, so there is no further durability step here.
    pub(super) fn sync_all(&mut self) -> Result<(), SnapshotError> {
        Ok(())
    }
}

/// Whether a path this run created can still be removed by name: it must
/// not have become a link or reparse point.
fn removable_entry(path: &Path) -> bool {
    match fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.file_type().is_symlink() => {
            !windows_identity::path_is_reparse_point(path).unwrap_or(true)
        }
        _ => false,
    }
}

/// Refuses every shape except empty, or exactly one regular-file `.git`
/// entry — git's own `--no-checkout` worktree pointer, never content. On
/// Windows the name compares case-insensitively, because `.GIT` is the same
/// file.
fn ensure_empty(root: &Path) -> Result<(), SnapshotError> {
    let mut entries = 0usize;
    let mut unexpected = false;
    for entry in
        fs::read_dir(root).map_err(|error| capture_io("list materialization target", &error))?
    {
        if entries == MAX_ROOT_ENTRIES {
            return Err(SnapshotError::MaterializeTargetNotEmpty {
                entries: MAX_ROOT_ENTRIES,
            });
        }
        let entry = entry.map_err(|error| capture_io("list materialization target", &error))?;
        let name = entry.file_name();
        let path = entry.path();
        entries += 1;
        if name.to_string_lossy().eq_ignore_ascii_case(".git") {
            let metadata =
                fs::symlink_metadata(&path).map_err(|_| SnapshotError::MaterializeTargetInvalid)?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(SnapshotError::MaterializeTargetInvalid);
            }
            if windows_identity::path_is_reparse_point(&path)
                .map_err(|_| SnapshotError::MaterializeTargetInvalid)?
            {
                return Err(SnapshotError::MaterializeTargetInvalid);
            }
        } else {
            unexpected = true;
        }
    }
    if unexpected {
        return Err(SnapshotError::MaterializeTargetNotEmpty { entries });
    }
    Ok(())
}
