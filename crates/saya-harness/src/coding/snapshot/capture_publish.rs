//! The no-replace publication rename for a staged capture. macOS
//! (`renameatx_np(RENAME_EXCL)`) and Linux (`renameat2(RENAME_NOREPLACE)`)
//! refuse a replacement in the kernel, so publication is atomic and
//! no-replace wherever the capture destination is supported at all; every
//! other target is refused upstream and has no branch here. Symbol names
//! and argument order are verified against `libc` 0.2.189, the version
//! pinned in `Cargo.lock`: `renameatx_np(fromfd, from, tofd, to, flags:
//! c_uint)` and `renameat2(olddirfd, oldpath, newdirfd, newpath, flags:
//! c_uint)`.
#![cfg(any(target_os = "macos", target_os = "linux"))]

use std::{ffi::OsStr, io, os::fd::RawFd};

use super::capture_dest_private::c_name;

/// Renames `old` (in `old_dir`) to `new` (in `new_dir`), refusing rather
/// than replacing any existing entry.
pub(super) fn rename_no_replace(
    old_dir: RawFd,
    old: &OsStr,
    new_dir: RawFd,
    new: &OsStr,
) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    return rename_excl(old_dir, old, new_dir, new);
    #[cfg(target_os = "linux")]
    return rename_noreplace(old_dir, old, new_dir, new);
}

/// macOS: `renameatx_np` with `RENAME_EXCL` refuses any existing target.
#[cfg(target_os = "macos")]
fn rename_excl(old_dir: RawFd, old: &OsStr, new_dir: RawFd, new: &OsStr) -> io::Result<()> {
    let old_c = c_name(old)?;
    let new_c = c_name(new)?;
    let rc = unsafe {
        libc::renameatx_np(
            old_dir,
            old_c.as_ptr(),
            new_dir,
            new_c.as_ptr(),
            libc::RENAME_EXCL,
        )
    };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Linux: `renameat2` with `RENAME_NOREPLACE` refuses any existing target.
#[cfg(target_os = "linux")]
fn rename_noreplace(old_dir: RawFd, old: &OsStr, new_dir: RawFd, new: &OsStr) -> io::Result<()> {
    let old_c = c_name(old)?;
    let new_c = c_name(new)?;
    let rc = unsafe {
        libc::renameat2(
            old_dir,
            old_c.as_ptr(),
            new_dir,
            new_c.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
