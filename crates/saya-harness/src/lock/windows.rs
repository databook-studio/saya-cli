//! Windows process liveness for the run lock: the same question the unix
//! `kill(pid, 0)` answers, asked of the process handle.

const ERROR_INVALID_PARAMETER: u32 = 87;
const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
const STILL_ACTIVE: u32 = 259;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn CloseHandle(handle: isize) -> i32;
    fn GetExitCodeProcess(handle: isize, code: *mut u32) -> i32;
    fn GetLastError() -> u32;
    fn OpenProcess(access: u32, inherit_handle: i32, process_id: u32) -> isize;
}

/// Whether `pid` names a process that exists right now. A process we may not
/// open is treated as live — the fail-closed rule the lock documents — and
/// only the explicit "no such process" error or a terminated exit code reports
/// it gone. A process that exits with code 259 is indistinguishable from a
/// live one through this API; that too resolves to "live", which refuses a
/// reclaim rather than taking a live holder's lock.
pub(super) fn process_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle == 0 {
        return unsafe { GetLastError() } != ERROR_INVALID_PARAMETER;
    }
    let mut code = 0u32;
    let queried = unsafe { GetExitCodeProcess(handle, &mut code) };
    let _ = unsafe { CloseHandle(handle) };
    queried == 0 || code == STILL_ACTIVE
}
