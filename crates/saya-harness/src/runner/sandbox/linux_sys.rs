//! The raw Linux syscall layer for the confinement: Landlock UAPI constants
//! and struct shapes, the version query, the ruleset calls, the namespace
//! and no-new-privs steps, and the proc-file writes — all `libc`-marshalled.
//!
//! **UNVERIFIED ON ANY LINUX HOST.** The syscall *numbers* come from
//! `libc` 0.2.189; the constants and struct shapes below are transcribed
//! from the kernel's `include/uapi/linux/landlock.h` and `prctl.h`
//! [UNVERIFIED — libc 0.2.189 does not export them on every gnu arch, so
//! this design carries its own]. Nothing here is a measured fact; the
//! enforcement canary in the probe is what decides.

// Landlock create_ruleset flags and rule types [UNVERIFIED transcription].
pub(super) const LANDLOCK_CREATE_RULESET_VERSION: u32 = 1;
pub(super) const LANDLOCK_RULE_PATH_BENEATH: u32 = 1;

// The filesystem access rights [UNVERIFIED transcription; ABI 1 rights are
// bits 0..=12, REFER is ABI 2 (Linux 5.19), TRUNCATE is ABI 3 (Linux 6.2)].
pub(super) const LANDLOCK_ACCESS_FS_EXECUTE: u64 = 1 << 0;
pub(super) const LANDLOCK_ACCESS_FS_WRITE_FILE: u64 = 1 << 1;
pub(super) const LANDLOCK_ACCESS_FS_READ_FILE: u64 = 1 << 2;
pub(super) const LANDLOCK_ACCESS_FS_READ_DIR: u64 = 1 << 3;
pub(super) const LANDLOCK_ACCESS_FS_REMOVE_DIR: u64 = 1 << 4;
pub(super) const LANDLOCK_ACCESS_FS_REMOVE_FILE: u64 = 1 << 5;
pub(super) const LANDLOCK_ACCESS_FS_MAKE_CHAR: u64 = 1 << 6;
pub(super) const LANDLOCK_ACCESS_FS_MAKE_DIR: u64 = 1 << 7;
pub(super) const LANDLOCK_ACCESS_FS_MAKE_REG: u64 = 1 << 8;
pub(super) const LANDLOCK_ACCESS_FS_MAKE_SOCK: u64 = 1 << 9;
pub(super) const LANDLOCK_ACCESS_FS_MAKE_FIFO: u64 = 1 << 10;
pub(super) const LANDLOCK_ACCESS_FS_MAKE_BLOCK: u64 = 1 << 11;
pub(super) const LANDLOCK_ACCESS_FS_MAKE_SYM: u64 = 1 << 12;
pub(super) const LANDLOCK_ACCESS_FS_REFER: u64 = 1 << 13;
pub(super) const LANDLOCK_ACCESS_FS_TRUNCATE: u64 = 1 << 14;

// prctl(PR_SET_NO_NEW_PRIVS) — required by landlock(7) for unprivileged use
// [UNVERIFIED transcription; libc 0.2.189 does not export the constant on
// every gnu arch].
pub(super) const PR_SET_NO_NEW_PRIVS: libc::c_int = 38;

// O_PATH — fcntl(2) 0o10000000 [UNVERIFIED transcription; libc's own value
// differs per arch in this locked version, so this design carries its own].
pub(super) const O_PATH: libc::c_int = 0o10000000;

/// `struct landlock_ruleset_attr` [UNVERIFIED shape].
#[repr(C)]
pub(super) struct LandlockRulesetAttr {
    pub(super) handled_access_fs: u64,
}

/// `struct landlock_path_beneath_attr` [UNVERIFIED shape: `__u64
/// allowed_access; __s32 parent_fd;` with tail padding].
#[repr(C)]
pub(super) struct LandlockPathBeneathAttr {
    pub(super) allowed_access: u64,
    pub(super) parent_fd: libc::c_int,
}

/// The Landlock ABI version the running kernel reports — the same raw
/// version query the spike probe recorded (syscall 444, version flag).
pub(super) fn landlock_abi() -> Result<u32, std::io::Error> {
    let r = unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            std::ptr::null::<libc::c_void>(),
            0usize,
            LANDLOCK_CREATE_RULESET_VERSION,
        )
    };
    if r == -1 {
        Err(last())
    } else {
        Ok(u32::try_from(r).unwrap_or(u32::MAX))
    }
}

pub(super) fn create_ruleset(handled_access_fs: u64) -> Result<libc::c_int, std::io::Error> {
    let attr = LandlockRulesetAttr { handled_access_fs };
    let r = unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            std::ptr::addr_of!(attr).cast::<libc::c_void>(),
            std::mem::size_of::<LandlockRulesetAttr>(),
            0u32,
        )
    };
    if r == -1 {
        Err(last())
    } else {
        Ok(r as libc::c_int)
    }
}

pub(super) fn add_path_beneath(
    ruleset_fd: libc::c_int,
    parent_fd: libc::c_int,
    allowed_access: u64,
) -> Result<(), std::io::Error> {
    let rule = LandlockPathBeneathAttr {
        allowed_access,
        parent_fd,
    };
    let r = unsafe {
        libc::syscall(
            libc::SYS_landlock_add_rule,
            ruleset_fd,
            LANDLOCK_RULE_PATH_BENEATH,
            std::ptr::addr_of!(rule).cast::<libc::c_void>(),
            0u32,
        )
    };
    if r == -1 { Err(last()) } else { Ok(()) }
}

pub(super) fn restrict_self(ruleset_fd: libc::c_int) -> Result<(), std::io::Error> {
    let r = unsafe { libc::syscall(libc::SYS_landlock_restrict_self, ruleset_fd, 0u32) };
    if r == -1 { Err(last()) } else { Ok(()) }
}

/// Opens `path` for the ruleset's PATH_BENEATH parent — `O_PATH` (no content
/// access, works on directories and unreadable paths) [UNVERIFIED shape].
pub(super) fn open_path(path: &std::ffi::CStr) -> Result<libc::c_int, std::io::Error> {
    let fd = unsafe { libc::open(path.as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC | O_PATH) };
    if fd == -1 { Err(last()) } else { Ok(fd) }
}

pub(super) fn no_new_privs() -> Result<(), std::io::Error> {
    let r = unsafe { libc::prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) };
    if r != 0 { Err(last()) } else { Ok(()) }
}

pub(super) fn unshare_user() -> Result<(), std::io::Error> {
    let r = unsafe { libc::unshare(libc::CLONE_NEWUSER) };
    if r != 0 { Err(last()) } else { Ok(()) }
}

pub(super) fn unshare_net() -> Result<(), std::io::Error> {
    let r = unsafe { libc::unshare(libc::CLONE_NEWNET) };
    if r != 0 { Err(last()) } else { Ok(()) }
}

/// Writes a `/proc` knob (setgroups, uid_map, gid_map) with the full content,
/// retrying short writes [UNVERIFIED per user_namespaces(7)].
pub(super) fn write_proc(path: &std::ffi::CStr, content: &str) -> Result<(), std::io::Error> {
    let fd = unsafe { libc::open(path.as_ptr(), libc::O_WRONLY | libc::O_CLOEXEC) };
    if fd == -1 {
        return Err(last());
    }
    let bytes = content.as_bytes();
    let mut written = 0usize;
    while written < bytes.len() {
        let n = unsafe {
            libc::write(
                fd,
                bytes[written..].as_ptr().cast::<libc::c_void>(),
                bytes.len() - written,
            )
        };
        if n <= 0 {
            unsafe { libc::close(fd) };
            return Err(last());
        }
        written += n as usize;
    }
    unsafe { libc::close(fd) };
    Ok(())
}

pub(super) fn getuid() -> u32 {
    unsafe { libc::getuid() }
}

pub(super) fn getgid() -> u32 {
    unsafe { libc::getgid() }
}

fn last() -> std::io::Error {
    std::io::Error::last_os_error()
}
