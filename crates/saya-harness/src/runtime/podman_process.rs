//! Bounded process lifetime and ticket handling for fixed Podman commands.

use std::{
    fs,
    io::Read as _,
    path::Path,
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

#[cfg(unix)]
use std::fs::OpenOptions;
#[cfg(unix)]
use std::os::unix::{fs::OpenOptionsExt as _, process::CommandExt as _};

use saya_agent::CancellationToken;

use super::podman::PodmanError;

static TICKET_COUNTER: AtomicU64 = AtomicU64::new(0);

pub(super) struct Output {
    pub(super) success: bool,
    pub(super) exit_code: Option<i32>,
    pub(super) pid: u32,
    pub(super) signal: Option<i32>,
    pub(super) timed_out: bool,
    pub(super) cancelled: bool,
    pub(super) killed_orphans: bool,
    pub(super) duration_ms: u64,
    pub(super) stdout: crate::runner::output::StreamCapture,
    pub(super) stderr: crate::runner::output::StreamCapture,
}

pub(super) async fn run(
    executable: &Path,
    home: Option<&Path>,
    authfile: &Path,
    arguments: &[&str],
    timeout: Duration,
    cancellation: &CancellationToken,
) -> Result<Output, PodmanError> {
    let mut command = Command::new(executable);
    command
        .args(arguments)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(home) = home {
        command.env("HOME", home);
    }
    command.env("REGISTRY_AUTH_FILE", authfile);
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command.spawn().map_err(|_| PodmanError::Unavailable)?;
    let pid = child.id();
    let mut group = ProcessGroup::new(pid);
    let pipes = match crate::proc::Pipes::take(&mut child.stdout, &mut child.stderr) {
        Ok(pipes) => pipes,
        Err(_) => {
            let _ = crate::proc::kill_process_group(pid);
            return match child.wait() {
                Ok(_) => {
                    group.disarm();
                    Err(PodmanError::CleanupUnknown { operation: None })
                }
                Err(_) => Err(PodmanError::CleanupUnknown { operation: None }),
            };
        }
    };
    let settled = match crate::proc::wait(child, timeout, cancellation).await {
        Ok(settled) => {
            group.disarm();
            settled
        }
        Err(_) => return Err(PodmanError::CleanupUnknown { operation: None }),
    };
    let (stdout, stderr) = pipes.finish(&[]);
    Ok(Output {
        success: settled.end == crate::proc::WaitEnd::Exited && settled.status.success(),
        exit_code: settled.status.code(),
        pid,
        signal: exit_signal(&settled.status),
        timed_out: settled.end == crate::proc::WaitEnd::TimedOut,
        cancelled: settled.end == crate::proc::WaitEnd::Cancelled,
        killed_orphans: settled.killed_orphans,
        duration_ms: settled.duration_ms,
        stdout,
        stderr,
    })
}

fn exit_signal(status: &std::process::ExitStatus) -> Option<i32> {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt as _;
        status.signal()
    }
    #[cfg(not(unix))]
    {
        let _ = status;
        None
    }
}

struct ProcessGroup {
    pid: u32,
    armed: bool,
}

impl ProcessGroup {
    fn new(pid: u32) -> Self {
        Self { pid, armed: true }
    }
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for ProcessGroup {
    fn drop(&mut self) {
        if self.armed {
            let _ = crate::proc::kill_process_group(self.pid);
        }
    }
}

pub(super) fn ticket() -> Result<String, PodmanError> {
    let mut bytes = [0u8; 16];
    fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut bytes))
        .map_err(|_| PodmanError::Failed)?;
    let sequence = TICKET_COUNTER.fetch_add(1, Ordering::Relaxed);
    Ok(format!(
        "{sequence:016x}{}",
        bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    ))
}

#[cfg(unix)]
pub(super) fn read_container_id(path: &Path) -> Result<String, ()> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
        .map_err(|_| ())?;
    let metadata = file.metadata().map_err(|_| ())?;
    if !metadata.is_file() || metadata.len() > 128 {
        return Err(());
    }
    let mut value = String::new();
    file.take(129).read_to_string(&mut value).map_err(|_| ())?;
    let value = value.trim();
    if value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(value.to_owned())
    } else {
        Err(())
    }
}

#[cfg(not(unix))]
pub(super) fn read_container_id(_: &Path) -> Result<String, ()> {
    Err(())
}
