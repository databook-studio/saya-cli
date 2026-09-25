//! The Linux canary fork plumbing: fork + pipe + bounded wait, the errno
//! report protocol, the loopback connect, and the three canary children (fs,
//! exec, namespace-only). The children perform no allocation after the fork.
//! Runs only on a Linux host, through exactly the code
//! [`RunnerSpawn`](super::spawn::RunnerSpawn) will use; the verdict reads its
//! result, never the assumptions in `linux.rs`. Every child is forked
//! because `unshare(CLONE_NEWUSER)` is refused in a multithreaded process
//! [UNVERIFIED per unshare(2)]; the harness is threaded.

use std::{
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

use super::linux::Confinement;

/// How long a canary child may run before the parent kills it.
const CHILD_TIMEOUT: Duration = Duration::from_secs(10);

/// What one canary child reported, as errno magnitudes: 0 = succeeded, the
/// negated errno = failed; a never-reported slot reads [`i32::MIN`] and the
/// check treats it as unattributed, never as evidence.
pub(super) struct CanaryResult {
    words: [i32; 4],
    exited: bool,
    code: Option<i32>,
}

impl CanaryResult {
    pub(super) fn slot(&self, i: usize) -> i32 {
        self.words.get(i).copied().unwrap_or(i32::MIN)
    }

    /// Whether the child exited cleanly.
    pub(super) fn exited(&self) -> bool {
        self.exited
    }

    /// The child's exit code, when it exited cleanly.
    pub(super) fn code(&self) -> Option<i32> {
        self.code
    }
}

/// Forks `child` with a fresh pipe; the child writes its report and exits
/// via `_exit`. Returns the pid and the parent's read end.
pub(super) fn fork_with_pipe(
    child: impl FnOnce(&[libc::c_int; 2]),
) -> Result<(libc::pid_t, libc::c_int), String> {
    let mut fds = [0 as libc::c_int; 2];
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return Err(format!("pipe: {}", std::io::Error::last_os_error()));
    }
    let pid = unsafe { libc::fork() };
    if pid == -1 {
        unsafe { libc::close(fds[0]) };
        unsafe { libc::close(fds[1]) };
        return Err(format!("fork: {}", std::io::Error::last_os_error()));
    }
    if pid == 0 {
        child(&fds);
        unsafe { libc::close(fds[1]) };
        unsafe { libc::_exit(0) };
    }
    unsafe { libc::close(fds[1]) };
    Ok((pid, fds[0]))
}

/// Waits for a canary child with a wall bound; a wedged child is killed,
/// never reported as evidence.
fn wait_canary(pid: libc::pid_t) -> (bool, Option<i32>) {
    let start = Instant::now();
    let mut status: libc::c_int = 0;
    loop {
        let r = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
        if r == pid {
            let exited = (status & 0x7f) == 0;
            let code = exited.then_some(((status >> 8) & 0xff) as i32);
            return (exited, code);
        }
        if r == -1 {
            return (false, None);
        }
        if start.elapsed() > CHILD_TIMEOUT {
            unsafe {
                libc::kill(pid, libc::SIGKILL);
                libc::waitpid(pid, std::ptr::null_mut(), 0);
            }
            return (false, None);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Reads up to four errno words a canary child reported; never-written
/// slots read as [`i32::MIN`] (absence).
pub(super) fn collect_canary(pid: libc::pid_t, fd: libc::c_int) -> CanaryResult {
    let (exited, code) = wait_canary(pid);
    let mut buf = [0u8; 16];
    let mut got = 0usize;
    while got < buf.len() {
        let n = unsafe { libc::read(fd, buf.as_mut_ptr().add(got).cast(), buf.len() - got) };
        if n <= 0 {
            break;
        }
        got += n as usize;
    }
    unsafe { libc::close(fd) };
    let words: [i32; 4] = std::array::from_fn(|i| {
        if got >= (i + 1) * 4 {
            i32::from_ne_bytes(buf[i * 4..i * 4 + 4].try_into().expect("word slice"))
        } else {
            i32::MIN
        }
    });
    CanaryResult {
        words,
        exited,
        code,
    }
}

/// Forks the fs canary: applies `confinement`, then mkdir inside the root
/// (must succeed), create outside the roots (must fail), and a loopback
/// connect through the netns (must fail). Slots: [apply, inside, outside,
/// connect].
pub(super) fn fork_fs_canary(
    confinement: Arc<Confinement>,
    inside: &Path,
    outside: &Path,
    connect_port: u16,
) -> Result<CanaryResult, String> {
    let inside = cstring(inside)?;
    let outside = cstring(outside)?;
    let (pid, fd) = fork_with_pipe(|fds| {
        let mut w = [0_i32; 4];
        match confinement.apply() {
            Err(e) => w[0] = -raw(&e),
            Ok(()) => {
                w[0] = 0;
                let made = unsafe { libc::mkdir(inside.as_ptr(), 0o700) };
                w[1] = if made == 0 { 0 } else { -raw_io() };
                let opened = unsafe {
                    libc::open(
                        outside.as_ptr(),
                        libc::O_CREAT | libc::O_EXCL | libc::O_WRONLY | libc::O_CLOEXEC,
                        0o600,
                    )
                };
                if opened == -1 {
                    w[2] = -raw_io();
                } else {
                    w[2] = 0;
                    unsafe { libc::close(opened) };
                }
                w[3] = match loopback_connect(connect_port) {
                    Ok(()) => 0,
                    Err(e) => -e,
                };
            }
        }
        write_words(fds[1], &w);
    })?;
    Ok(collect_canary(pid, fd))
}

/// Forks the exec canary: applies the confinement and execs a canary binary
/// from the program directory. Success is the exec'd binary's own clean exit
/// — exec replaced the child, so a report word exists only on failure.
/// Slots: [apply, exec].
pub(super) fn fork_exec_canary(
    confinement: Arc<Confinement>,
    program: &Path,
) -> Result<CanaryResult, String> {
    let program = cstring(program)?;
    let (pid, fd) = fork_with_pipe(|fds| {
        let mut w = [0_i32; 4];
        match confinement.apply() {
            Err(e) => w[0] = -raw(&e),
            Ok(()) => {
                let argv: [*const libc::c_char; 2] = [program.as_ptr(), std::ptr::null()];
                if unsafe { libc::execv(program.as_ptr(), argv.as_ptr()) } == -1 {
                    w[1] = -raw_io();
                }
            }
        }
        if w[0] != 0 || w[1] != 0 {
            write_words(fds[1], &w);
        }
    })?;
    Ok(collect_canary(pid, fd))
}

pub(super) fn write_words(fd: libc::c_int, words: &[i32; 4]) {
    let bytes = words.as_ptr().cast::<libc::c_void>();
    unsafe {
        libc::write(
            fd,
            bytes,
            (words.len() * std::mem::size_of::<i32>()) as libc::size_t,
        )
    };
}

fn loopback_connect(port: u16) -> Result<(), i32> {
    unsafe {
        let fd = libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0);
        if fd == -1 {
            return Err(raw_io());
        }
        let addr = libc::sockaddr_in {
            sin_family: libc::AF_INET as libc::sa_family_t,
            sin_port: port.to_be(),
            sin_addr: libc::in_addr {
                s_addr: u32::from_ne_bytes([127, 0, 0, 1]),
            },
            sin_zero: [0; 8],
        };
        let r = libc::connect(
            fd,
            std::ptr::addr_of!(addr).cast::<libc::sockaddr>(),
            std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
        );
        let errno = if r == -1 { raw_io() } else { 0 };
        libc::close(fd);
        if errno == 0 { Ok(()) } else { Err(errno) }
    }
}

fn raw(e: &std::io::Error) -> i32 {
    e.raw_os_error().unwrap_or(0)
}

pub(super) fn raw_io() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

fn cstring(path: &Path) -> Result<std::ffi::CString, String> {
    std::ffi::CString::new(path.as_os_str().as_encoded_bytes())
        .map_err(|_| "a canary path contains NUL".to_owned())
}
