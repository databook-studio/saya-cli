//! Private storage paths and no-replace publication for verified npm inputs.

use std::{
    fs,
    path::{Path, PathBuf},
};

use super::{Registry, RegistryError, private_fs, storage};

impl Registry {
    pub(crate) fn npm_input_root(&self) -> Result<PathBuf, RegistryError> {
        let root = self.root.join("npm-inputs");
        storage::ensure_private_dir(&root, true)?;
        Ok(root)
    }

    pub(crate) fn create_npm_stage(&self, ticket: &str) -> Result<PathBuf, RegistryError> {
        if !valid_hex(ticket, 48) {
            return Err(RegistryError::Conflict);
        }
        let root = self.npm_input_root()?;
        let stage = root.join(format!("stage-{ticket}"));
        private_fs::create_private_dir(&stage).map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                RegistryError::Conflict
            } else {
                RegistryError::Io
            }
        })?;
        private_fs::sync_directory(&root).map_err(|_| RegistryError::Io)?;
        Ok(stage)
    }

    pub(crate) fn publish_npm_bundle(
        &self,
        stage: &Path,
        input_key: &str,
    ) -> Result<PathBuf, RegistryError> {
        if !valid_hex(input_key, 64) {
            return Err(RegistryError::Conflict);
        }
        let root = self.npm_input_root()?;
        if stage.parent() != Some(root.as_path())
            || !stage
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("stage-"))
        {
            return Err(RegistryError::Conflict);
        }
        private_fs::check_private_dir(stage).map_err(|_| RegistryError::Conflict)?;
        let final_path = root.join(input_key);
        if fs::symlink_metadata(&final_path).is_ok() {
            return Err(RegistryError::Conflict);
        }
        // Registry::open holds the cross-process owner lock, so this same-filesystem
        // directory rename publishes the complete bundle in one step.
        fs::rename(stage, &final_path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                RegistryError::Conflict
            } else {
                RegistryError::Io
            }
        })?;
        private_fs::sync_directory(&final_path).map_err(|_| RegistryError::Conflict)?;
        private_fs::sync_directory(&root).map_err(|_| RegistryError::Conflict)?;
        Ok(final_path)
    }
}

fn valid_hex(value: &str, len: usize) -> bool {
    value.len() == len && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}
