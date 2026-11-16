//! Staged auth and ready records for the private runtime registry.

use std::{
    fs,
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use super::{RegistryError, private_fs};

pub(super) const READY_STAGE: &str = "ready-stage-";
const AUTH_STAGE: &str = "auth-stage-";
const EMPTY_AUTH: &str = "{\"auths\":{}}\n";
const RECORD_LIMIT: u64 = 512;
static STAGE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(super) fn expected_record() -> String {
    format!(
        "image={}\nnode={}\nnpm={}\n",
        super::NODE_IMAGE_MANIFEST_DIGEST,
        super::NODE_VERSION,
        super::NPM_VERSION
    )
}

pub(super) fn read_private_file(path: &Path) -> Result<Option<String>, RegistryError> {
    let file = match private_fs::open_read(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) if error.kind() == std::io::ErrorKind::InvalidData => {
            return Err(RegistryError::Conflict);
        }
        Err(_) => return Err(RegistryError::Io),
    };
    let mut value = String::new();
    file.take(RECORD_LIMIT + 1)
        .read_to_string(&mut value)
        .map_err(|_| RegistryError::Conflict)?;
    if value.len() as u64 > RECORD_LIMIT {
        return Err(RegistryError::Conflict);
    }
    Ok(Some(value))
}

pub(super) fn ensure_private_dir(path: &Path, create: bool) -> Result<(), RegistryError> {
    if create {
        match private_fs::create_private_dir(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => return Err(RegistryError::Io),
        }
    }
    private_fs::check_private_dir(path).map_err(|_| RegistryError::InvalidStateRoot)?;
    private_fs::sync_directory(path.parent().ok_or(RegistryError::Io)?)
        .map_err(|_| RegistryError::Io)
}

pub(super) fn ensure_auth_file(path: &Path) -> Result<(), RegistryError> {
    if let Some(value) = read_private_file(path)? {
        return if valid_empty_auth(&value) {
            Ok(())
        } else {
            Err(RegistryError::Conflict)
        };
    }
    let (stage, mut file) = staged_file(path.parent().ok_or(RegistryError::Io)?, AUTH_STAGE)?;
    if file
        .write_all(EMPTY_AUTH.as_bytes())
        .and_then(|()| file.sync_all())
        .is_err()
    {
        let _ = fs::remove_file(stage);
        return Err(RegistryError::Io);
    }
    if let Err(error) = fs::hard_link(&stage, path) {
        let _ = fs::remove_file(stage);
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            let Some(value) = read_private_file(path)? else {
                return Err(RegistryError::Conflict);
            };
            return if valid_empty_auth(&value) {
                Ok(())
            } else {
                Err(RegistryError::Conflict)
            };
        }
        return Err(RegistryError::Io);
    }
    let parent = path.parent().ok_or(RegistryError::Io)?;
    private_fs::sync_directory(parent).map_err(|_| RegistryError::Io)?;
    fs::remove_file(stage).map_err(|_| RegistryError::Io)?;
    private_fs::sync_directory(parent).map_err(|_| RegistryError::Io)
}

fn valid_empty_auth(value: &str) -> bool {
    let Ok(document) = serde_json::from_str::<serde_json::Value>(value) else {
        return false;
    };
    let Some(fields) = document.as_object() else {
        return false;
    };
    fields.len() == 1
        && fields
            .get("auths")
            .is_some_and(|auths| auths.as_object().is_some_and(|map| map.is_empty()))
}

pub(super) fn cleanup_staged_files(root: &Path) -> Result<(), RegistryError> {
    let entries = fs::read_dir(root).map_err(|_| RegistryError::Io)?;
    let mut removed = false;
    for entry in entries {
        let entry = entry.map_err(|_| RegistryError::Io)?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let final_name = if name.starts_with(READY_STAGE) {
            super::READY
        } else if name.starts_with(AUTH_STAGE) {
            super::AUTH
        } else {
            continue;
        };
        private_fs::check_private_stage(&entry.path(), &root.join(final_name))
            .map_err(|_| RegistryError::Conflict)?;
        fs::remove_file(entry.path()).map_err(|_| RegistryError::Io)?;
        removed = true;
    }
    if removed {
        private_fs::sync_directory(root).map_err(|_| RegistryError::Io)?;
    }
    Ok(())
}

pub(super) fn staged_file(root: &Path, prefix: &str) -> Result<(PathBuf, fs::File), RegistryError> {
    for _ in 0..32 {
        let sequence = STAGE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let name = format!("{prefix}{}-{sequence}", std::process::id());
        let path = root.join(name);
        match private_fs::create_private_file(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return Err(RegistryError::Io),
        }
    }
    Err(RegistryError::Io)
}
