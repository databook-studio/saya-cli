//! Safe creation and complete ancestor validation for the state leaf.

#[cfg(unix)]
use std::os::unix::fs::MetadataExt as _;
use std::{
    fs,
    path::{Component, Path, PathBuf},
};

const MAX_MISSING_COMPONENTS: usize = 8;

pub(super) fn ensure(path: &Path) -> Result<PathBuf, super::super::RegistryError> {
    match existing(path)? {
        Some(path) => Ok(path),
        None => create_missing_suffix(path),
    }
}

pub(super) fn existing(path: &Path) -> Result<Option<PathBuf>, super::super::RegistryError> {
    if !path.is_absolute() {
        return Err(super::super::RegistryError::InvalidStateRoot);
    }
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if !super::private_dir(&metadata) {
                return Err(super::super::RegistryError::InvalidStateRoot);
            }
            check_parent(path)?;
            fs::canonicalize(path)
                .map(Some)
                .map_err(|_| super::super::RegistryError::InvalidStateRoot)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(super::super::RegistryError::InvalidStateRoot),
    }
}

fn create_missing_suffix(path: &Path) -> Result<PathBuf, super::super::RegistryError> {
    if path
        .components()
        .any(|part| matches!(part, Component::CurDir | Component::ParentDir))
    {
        return Err(super::super::RegistryError::InvalidStateRoot);
    }
    let mut missing = Vec::new();
    let mut anchor = path;
    let metadata = loop {
        match fs::symlink_metadata(anchor) {
            Ok(metadata) => break metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if !matches!(anchor.components().next_back(), Some(Component::Normal(_))) {
                    return Err(super::super::RegistryError::InvalidStateRoot);
                }
                missing.push(
                    anchor
                        .file_name()
                        .ok_or(super::super::RegistryError::InvalidStateRoot)?
                        .to_os_string(),
                );
                if missing.len() > MAX_MISSING_COMPONENTS {
                    return Err(super::super::RegistryError::InvalidStateRoot);
                }
                anchor = anchor
                    .parent()
                    .ok_or(super::super::RegistryError::InvalidStateRoot)?;
            }
            Err(_) => return Err(super::super::RegistryError::InvalidStateRoot),
        }
    };
    if !metadata.is_dir() || metadata.file_type().is_symlink() || !trusted_anchor(&metadata) {
        return Err(super::super::RegistryError::InvalidStateRoot);
    }
    let canonical =
        fs::canonicalize(anchor).map_err(|_| super::super::RegistryError::InvalidStateRoot)?;
    if canonical != anchor || !trusted_ancestors(&canonical) {
        return Err(super::super::RegistryError::InvalidStateRoot);
    }
    let mut current = canonical;
    for name in missing.into_iter().rev() {
        let next = current.join(name);
        match super::create_private_dir(&next) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => return Err(super::super::RegistryError::Io),
        }
        super::check_private_dir(&next)
            .map_err(|_| super::super::RegistryError::InvalidStateRoot)?;
        super::sync_directory(&current).map_err(|_| super::super::RegistryError::Io)?;
        current = next;
    }
    Ok(current)
}

fn check_parent(path: &Path) -> Result<PathBuf, super::super::RegistryError> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or(super::super::RegistryError::InvalidStateRoot)?;
    let canonical =
        fs::canonicalize(parent).map_err(|_| super::super::RegistryError::InvalidStateRoot)?;
    if canonical.as_path() != parent || !trusted_ancestors(&canonical) {
        return Err(super::super::RegistryError::InvalidStateRoot);
    }
    Ok(canonical)
}

fn trusted_ancestors(path: &Path) -> bool {
    path.ancestors().all(|ancestor| {
        fs::symlink_metadata(ancestor).is_ok_and(|metadata| {
            safe_parent(&metadata) || trusted_sticky_temp(ancestor, &metadata)
        })
    })
}

#[cfg(unix)]
fn safe_parent(metadata: &fs::Metadata) -> bool {
    metadata.is_dir()
        && !metadata.file_type().is_symlink()
        && metadata.mode() & 0o022 == 0
        && (metadata.uid() == 0 || metadata.uid() == unsafe { libc::geteuid() })
}
#[cfg(not(unix))]
fn safe_parent(_: &fs::Metadata) -> bool {
    false
}

#[cfg(unix)]
fn trusted_anchor(metadata: &fs::Metadata) -> bool {
    safe_parent(metadata) && metadata.uid() == unsafe { libc::geteuid() }
}
#[cfg(not(unix))]
fn trusted_anchor(_: &fs::Metadata) -> bool {
    false
}

#[cfg(unix)]
fn trusted_sticky_temp(path: &Path, metadata: &fs::Metadata) -> bool {
    metadata.is_dir()
        && !metadata.file_type().is_symlink()
        && metadata.uid() == 0
        && metadata.mode() & crate::runtime::STICKY_BIT != 0
        && ["/tmp", "/var/tmp", "/private/tmp", "/private/var/tmp"]
            .iter()
            .any(|trusted| path == Path::new(trusted))
}
#[cfg(not(unix))]
fn trusted_sticky_temp(_: &Path, _: &fs::Metadata) -> bool {
    false
}
