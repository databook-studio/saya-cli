//! Fixed Podman locations and trusted Homebrew executable resolution.

use std::{
    env, fs,
    path::{Path, PathBuf},
};

#[cfg(unix)]
use std::os::unix::fs::MetadataExt as _;

const HOMEBREW_BIN: &str = "/opt/homebrew/bin/podman";
const HOMEBREW_CELLAR: &str = "/opt/homebrew/Cellar/podman";

#[cfg(test)]
#[path = "podman_discovery_tests.rs"]
mod tests;

pub(super) fn trusted_home() -> Option<PathBuf> {
    env::var_os("HOME")
        .map(PathBuf::from)
        .and_then(trusted_home_path)
}

pub(super) fn trusted_executable(path: &Path) -> Option<PathBuf> {
    let link = fs::symlink_metadata(path).ok()?;
    let target = if link.file_type().is_symlink() {
        if path != Path::new(HOMEBREW_BIN) {
            return None;
        }
        let target = fs::canonicalize(path).ok()?;
        if !target.starts_with(HOMEBREW_CELLAR) || !trusted_directories(path.parent()?) {
            return None;
        }
        target
    } else {
        fs::canonicalize(path).ok()?
    };
    let metadata = fs::symlink_metadata(&target).ok()?;
    if !trusted_file(&metadata) || !trusted_directories(target.parent()?) {
        return None;
    }
    Some(target)
}

pub(super) fn candidates() -> Vec<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        vec![
            PathBuf::from(HOMEBREW_BIN),
            PathBuf::from("/opt/podman/bin/podman"),
            PathBuf::from("/usr/local/bin/podman"),
        ]
    }
    #[cfg(target_os = "linux")]
    {
        vec![
            PathBuf::from("/usr/bin/podman"),
            PathBuf::from("/usr/local/bin/podman"),
        ]
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        Vec::new()
    }
}

#[cfg(unix)]
fn trusted_file(metadata: &fs::Metadata) -> bool {
    metadata.is_file()
        && !metadata.file_type().is_symlink()
        && metadata.mode() & 0o022 == 0
        && metadata.mode() & 0o111 != 0
        && (metadata.uid() == 0 || metadata.uid() == unsafe { libc::geteuid() })
}
#[cfg(not(unix))]
fn trusted_file(_: &fs::Metadata) -> bool {
    false
}

fn trusted_home_path(path: PathBuf) -> Option<PathBuf> {
    let metadata = fs::symlink_metadata(&path).ok()?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() || !trusted_directories(&path) {
        return None;
    }
    #[cfg(unix)]
    if metadata.uid() != unsafe { libc::geteuid() } {
        return None;
    }
    fs::canonicalize(path).ok()
}

fn trusted_directories(path: &Path) -> bool {
    let Ok(canonical) = fs::canonicalize(path) else {
        return false;
    };
    if canonical != path {
        return false;
    }
    for directory in canonical.ancestors() {
        let Ok(metadata) = fs::symlink_metadata(directory) else {
            return false;
        };
        if !trusted_directory(directory, &metadata) {
            return false;
        }
    }
    true
}

#[cfg(unix)]
fn trusted_directory(path: &Path, metadata: &fs::Metadata) -> bool {
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return false;
    }
    let uid = metadata.uid();
    let mode = metadata.mode();
    (uid == 0 || uid == unsafe { libc::geteuid() })
        && (mode & 0o022 == 0
            || (uid == 0 && mode & crate::runtime::STICKY_BIT != 0)
            || trusted_homebrew_directory(path, metadata))
}
#[cfg(not(unix))]
fn trusted_directory(_: &Path, _: &fs::Metadata) -> bool {
    false
}

#[cfg(target_os = "macos")]
fn trusted_homebrew_directory(path: &Path, metadata: &fs::Metadata) -> bool {
    let Some(admin_gid) = admin_gid() else {
        return false;
    };
    path.starts_with("/opt/homebrew")
        && (metadata.uid() == 0 || metadata.uid() == unsafe { libc::geteuid() })
        && metadata.gid() == admin_gid
        && metadata.mode() & 0o020 != 0
        && metadata.mode() & 0o002 == 0
}

#[cfg(all(unix, not(target_os = "macos")))]
fn trusted_homebrew_directory(_: &Path, _: &fs::Metadata) -> bool {
    false
}

#[cfg(target_os = "macos")]
fn admin_gid() -> Option<u32> {
    let name = std::ffi::CString::new("admin").ok()?;
    let group = unsafe { libc::getgrnam(name.as_ptr()) };
    if group.is_null() {
        None
    } else {
        Some(unsafe { (*group).gr_gid as u32 })
    }
}
