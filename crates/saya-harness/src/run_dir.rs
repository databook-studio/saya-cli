//! The on-disk layout of a run: `runs/<id>/` with its `workspace/` and
//! `state/` subdirectories, created by the engine at 0700 — never inferred
//! from the current directory. The run id is a [`RunId`], whose shape is
//! already restricted to characters safe to embed in a path component.

use std::{
    fs,
    path::{Path, PathBuf},
};

use saya_types::RunId;

use crate::{HarnessError, io_error};

/// The created layout of one run's directory.
#[derive(Debug, Clone)]
pub struct RunDir {
    root: PathBuf,
    workspace: PathBuf,
    state: PathBuf,
}

impl RunDir {
    /// Creates (or re-enters, on resume) `runs/<id>/` and its `workspace/`
    /// and `state/` subdirectories, setting each to 0700 on every call so a
    /// loose mode cannot survive a resume.
    pub fn create(runs_root: &Path, id: &RunId) -> Result<Self, HarnessError> {
        let root = runs_root.join(id.as_str());
        ensure_dir(runs_root, "create runs root")?;
        ensure_dir(&root, "create run dir")?;
        let workspace = root.join("workspace");
        ensure_dir(&workspace, "create run workspace")?;
        let state = root.join("state");
        ensure_dir(&state, "create run state dir")?;
        Ok(Self {
            root,
            workspace,
            state,
        })
    }

    /// The run's own directory, `runs/<id>/`.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Where the run's artifacts live.
    pub fn workspace(&self) -> &Path {
        &self.workspace
    }

    /// Where generated config and spawned-process state live.
    pub fn state(&self) -> &Path {
        &self.state
    }

    /// Where the engine's single-writer lock lives.
    pub fn lock_file(&self) -> PathBuf {
        self.root.join("lock")
    }
}

fn ensure_dir(path: &Path, context: &str) -> Result<(), HarnessError> {
    fs::create_dir_all(path).map_err(|error| io_error(context, path, error))?;
    #[cfg(unix)]
    set_mode(path, 0o700)?;
    Ok(())
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> Result<(), HarnessError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .map_err(|error| io_error("set mode on", path, error))
}
