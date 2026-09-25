//! Windows process-tree termination for the shared process wait path.

use std::{ffi::OsString, io, os::windows::ffi::OsStringExt as _, path::PathBuf, process::Command};

const ERROR_INVALID_PARAMETER: u32 = 87;
const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn CloseHandle(handle: isize) -> i32;
    fn GetLastError() -> u32;
    fn GetSystemDirectoryW(buffer: *mut u16, size: u32) -> u32;
    fn OpenProcess(access: u32, inherit_handle: i32, process_id: u32) -> isize;
}

/// Kills `pid` and descendants through the Windows-supplied `taskkill.exe`.
/// A nonzero `taskkill` result is accepted only if the child exited in the
/// race before the command inspected it; other cleanup failures are returned
/// so the timeout path does not wait forever for a still-running child.
pub(super) fn kill_process_tree(pid: u32) -> io::Result<()> {
    let taskkill = system_taskkill().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "Windows system taskkill.exe is unavailable",
        )
    })?;
    let status = Command::new(taskkill)
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .status()?;
    if status.success() || process_is_gone(pid) {
        return Ok(());
    }
    Err(io::Error::other(format!(
        "taskkill failed while terminating process tree {pid}: {status}"
    )))
}

/// Locates `taskkill.exe` through the Windows API, never `PATH` or the
/// current directory, both of which a host-lane caller can influence.
fn system_taskkill() -> Option<PathBuf> {
    let mut buffer = vec![0u16; 32_768];
    let length = unsafe { GetSystemDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) };
    if length == 0 || length as usize >= buffer.len() {
        return None;
    }
    Some(PathBuf::from(OsString::from_wide(&buffer[..length as usize])).join("taskkill.exe"))
}

/// `taskkill` returns nonzero when a concurrently exiting process has already
/// disappeared. Distinguish that harmless race from a live process that the
/// command failed to terminate.
fn process_is_gone(pid: u32) -> bool {
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle == 0 {
        return unsafe { GetLastError() } == ERROR_INVALID_PARAMETER;
    }
    let _ = unsafe { CloseHandle(handle) };
    false
}
