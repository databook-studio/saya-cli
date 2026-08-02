use crate::StoreError;
use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

pub(crate) fn prepare_path(path: &Path) -> Result<(), StoreError> {
    let parent = path.parent().ok_or(StoreError::Unavailable)?;
    if parent.parent().is_none() || parent == std::env::temp_dir() {
        return Err(StoreError::Unavailable);
    }
    fs::create_dir_all(parent).map_err(|_| StoreError::Unavailable)?;
    #[cfg(unix)]
    set_mode(parent, 0o700)?;
    Ok(())
}

pub(crate) fn secure_files(path: &Path) -> Result<(), StoreError> {
    #[cfg(unix)]
    for suffix in ["", "-wal", "-shm"] {
        let sidecar = state_sidecar_path(path, suffix);
        if sidecar.exists() {
            set_mode(&sidecar, 0o600)?;
        }
    }
    Ok(())
}

pub fn state_sidecar_path(path: &Path, suffix: &str) -> PathBuf {
    let mut value: OsString = path.as_os_str().to_os_string();
    value.push(suffix);
    value.into()
}

pub(crate) fn validate_profile_id(value: &str) -> Result<(), StoreError> {
    if value.len() == 66
        && value.starts_with("p-")
        && value[2..].bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        Ok(())
    } else {
        Err(StoreError::Unavailable)
    }
}
pub(crate) fn validate_session_id(value: &str) -> Result<(), StoreError> {
    if !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        Ok(())
    } else {
        Err(StoreError::Unavailable)
    }
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> Result<(), StoreError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).map_err(|_| StoreError::Unavailable)
}
