//! The durable private record for one verified managed runtime.

#[path = "registry_npm.rs"]
mod npm_storage;
#[path = "registry_operations.rs"]
mod operations;
#[path = "registry_fs.rs"]
mod private_fs;
#[path = "registry_storage.rs"]
mod storage;

use std::{
    fs,
    io::Write as _,
    path::{Path, PathBuf},
};

#[cfg(unix)]
use std::os::fd::AsRawFd as _;

use super::{NODE_IMAGE_MANIFEST_DIGEST, NODE_VERSION, NPM_VERSION};

const DIRECTORY: &str = "managed-node-runtime";
const LOCK: &str = "install.lock";
const AUTH: &str = "registry-auth.json";
const READY: &str = "ready";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RegistryError {
    InvalidStateRoot,
    #[cfg(unix)]
    Busy,
    Conflict,
    Io,
}

pub(crate) struct Registry {
    root: PathBuf,
    _lock: Lock,
}

impl Registry {
    pub(crate) fn installation_recorded(state_root: &Path) -> Result<bool, RegistryError> {
        let Some(state_root) = private_fs::existing_state_root(state_root)? else {
            return Ok(false);
        };
        let root = state_root.join(DIRECTORY);
        match private_fs::check_private_dir(&root) {
            Ok(()) => Self::ready_at(&root),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(_) => Err(RegistryError::InvalidStateRoot),
        }
    }

    pub(crate) fn open(state_root: &Path) -> Result<Self, RegistryError> {
        let state_root = private_fs::ensure_state_root(state_root)?;
        let root = state_root.join(DIRECTORY);
        storage::ensure_private_dir(&root, true)?;
        let lock = Lock::claim(root.join(LOCK))?;
        storage::cleanup_staged_files(&root)?;
        storage::ensure_auth_file(&root.join(AUTH))?;
        Ok(Self { root, _lock: lock })
    }

    pub(crate) fn authfile(&self) -> PathBuf {
        self.root.join(AUTH)
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn begin_probe(&self, ticket: &str) -> Result<(), RegistryError> {
        operations::begin(&self.root, ticket)
    }

    pub(crate) fn finish_probe(&self, ticket: &str) -> Result<(), RegistryError> {
        operations::finish(&self.root, ticket)
    }

    pub(crate) fn pending_probe_tickets(&self) -> Result<Vec<String>, RegistryError> {
        operations::pending(&self.root)
    }

    pub(crate) fn has_ready(&self) -> Result<bool, RegistryError> {
        Self::ready_at(&self.root)
    }

    fn ready_at(root: &Path) -> Result<bool, RegistryError> {
        match storage::read_private_file(&root.join(READY))? {
            None => Ok(false),
            Some(record) if record == storage::expected_record() => Ok(true),
            Some(_) => Err(RegistryError::Conflict),
        }
    }

    /// Publishes a fully written record without replacing an existing one.
    pub(crate) fn publish(&self) -> Result<(), RegistryError> {
        let (stage, mut file) = storage::staged_file(&self.root, storage::READY_STAGE)?;
        let write_succeeded = file
            .write_all(storage::expected_record().as_bytes())
            .and_then(|()| file.sync_all())
            .is_ok();
        if !write_succeeded {
            let _ = fs::remove_file(stage);
            return Err(RegistryError::Io);
        }
        let ready = self.root.join(READY);
        if let Err(error) = fs::hard_link(&stage, &ready) {
            let _ = fs::remove_file(stage);
            return Err(if error.kind() == std::io::ErrorKind::AlreadyExists {
                RegistryError::Conflict
            } else {
                RegistryError::Io
            });
        }
        private_fs::sync_directory(&self.root).map_err(|_| RegistryError::Io)?;
        fs::remove_file(stage).map_err(|_| RegistryError::Io)?;
        private_fs::sync_directory(&self.root).map_err(|_| RegistryError::Io)
    }
}

struct Lock {
    _file: fs::File,
}

impl Lock {
    fn claim(path: PathBuf) -> Result<Self, RegistryError> {
        let file = private_fs::open_lock(&path).map_err(|_| RegistryError::Conflict)?;
        #[cfg(unix)]
        {
            // The file is persistent: flock is released by the OS after a crash,
            // unlike a create-new marker that leaves a permanent stale lock.
            let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
            if result != 0 {
                let error = std::io::Error::last_os_error();
                return Err(if error.raw_os_error() == Some(libc::EWOULDBLOCK) {
                    RegistryError::Busy
                } else {
                    RegistryError::Io
                });
            }
        }
        private_fs::sync_directory(path.parent().ok_or(RegistryError::Io)?)
            .map_err(|_| RegistryError::Io)?;
        Ok(Self { _file: file })
    }
}
