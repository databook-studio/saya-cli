use crate::StoreError;
use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

pub(crate) fn prepare_path(path: &Path) -> Result<(), StoreError> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    #[cfg(unix)]
    let existed = parent.exists();
    // A directory that cannot be created here can never be created: a parent
    // component is a regular file, the filesystem denies access, or it is
    // read-only. That is a permanent condition, not the transient write-lock
    // contention `Unavailable` exists for, so it is reported as `OpenFailed`
    // and the opener fails fast instead of retrying an unopenable path for the
    // full busy ceiling.
    fs::create_dir_all(parent).map_err(|_| StoreError::OpenFailed)?;
    #[cfg(unix)]
    if !existed {
        set_mode(parent, 0o700)?;
    }
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
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

pub fn state_sidecar_path(path: &Path, suffix: &str) -> PathBuf {
    let mut value: OsString = path.as_os_str().to_os_string();
    value.push(suffix);
    value.into()
}

// Values also arrive from disk and from callers outside this workspace, so the
// boundary check stays even though saya_types::ProfileIdentity now owns the rule.
//
// This is fractionally stricter than the check it replaces: the old one used
// is_ascii_hexdigit and so accepted uppercase, while ProfileIdentity admits only
// lowercase. Nothing stored is affected — the deriver has always emitted `{:02x}`
// — and one canonical spelling is what makes the identity usable as a key.
pub(crate) fn validate_profile_id(value: &str) -> Result<(), StoreError> {
    saya_types::ProfileIdentity::parse(value)
        .map(|_| ())
        .map_err(|_| StoreError::Invalid)
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
