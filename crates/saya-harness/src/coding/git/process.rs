use std::{
    io::Read,
    path::Path,
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use saya_agent::CancellationToken;

use super::{
    command::{GitError, GitOperation},
    environment::GitEnvironment,
    process_group::GitProcessGroup,
    request::GitRequest,
};

const PUMP_CHUNK_BYTES: usize = 8 * 1024;

pub(super) struct GitOutput {
    pub(super) exit_code: Option<i32>,
    pub(super) stdout: Vec<u8>,
}

pub(super) async fn execute(
    environment: &GitEnvironment,
    current_dir: &Path,
    request: GitRequest<'_>,
    cancellation: &CancellationToken,
) -> Result<GitOutput, GitError> {
    let operation = request.operation();
    let (stdout_limit, stderr_limit) = request.output_limits();
    let mut command = Command::new(&environment.executable);
    environment.apply(&mut command);
    command
        .arg("--no-lazy-fetch")
        .args(request.arguments())
        .current_dir(current_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        command.process_group(0);
    }
    let mut child = command.spawn().map_err(GitError::Spawn)?;
    let pid = child.id();
    let mut process_group = GitProcessGroup::new(pid);
    let Some(stdout_pipe) = child.stdout.take() else {
        let _ = crate::proc::kill_process_group(pid);
        let _ = child.wait();
        process_group.disarm();
        return Err(GitError::CaptureUnavailable);
    };
    let Some(stderr_pipe) = child.stderr.take() else {
        let _ = crate::proc::kill_process_group(pid);
        let _ = child.wait();
        process_group.disarm();
        return Err(GitError::CaptureUnavailable);
    };

    let stdout = Arc::new(Capture::new(stdout_limit));
    let stderr = Arc::new(Capture::new(stderr_limit));
    let stdout_reader = start_pump(Arc::clone(&stdout), stdout_pipe, pid);
    let stderr_reader = start_pump(Arc::clone(&stderr), stderr_pipe, pid);
    let settled = match crate::proc::wait(child, operation.timeout(), cancellation).await {
        Ok(settled) => {
            process_group.disarm();
            settled
        }
        Err(error) => return Err(GitError::Wait(error)),
    };
    let stdout_result = stdout_reader.await;
    let stderr_result = stderr_reader.await;
    if !matches!(stdout_result, Ok(Ok(()))) || !matches!(stderr_result, Ok(Ok(()))) {
        return Err(GitError::CaptureUnavailable);
    }
    if stdout.overflowed() || stderr.overflowed() {
        return Err(GitError::OutputLimitExceeded);
    }
    match settled.end {
        crate::proc::WaitEnd::TimedOut => Err(GitError::TimedOut),
        crate::proc::WaitEnd::Cancelled => Err(GitError::Cancelled),
        crate::proc::WaitEnd::Exited => Ok(GitOutput {
            exit_code: settled.status.code(),
            stdout: stdout.take_bytes(),
        }),
    }
}

fn start_pump<R: Read + Send + 'static>(
    capture: Arc<Capture>,
    reader: R,
    pid: u32,
) -> tokio::task::JoinHandle<std::io::Result<()>> {
    tokio::task::spawn_blocking(move || capture.pump(reader, pid))
}

struct Capture {
    bytes: Mutex<Vec<u8>>,
    maximum: usize,
    overflowed: AtomicBool,
}

impl Capture {
    fn new(maximum: usize) -> Self {
        Self {
            bytes: Mutex::new(Vec::new()),
            maximum,
            overflowed: AtomicBool::new(false),
        }
    }

    fn pump<R: Read>(&self, mut reader: R, pid: u32) -> std::io::Result<()> {
        let mut buffer = [0u8; PUMP_CHUNK_BYTES];
        loop {
            let read = match reader.read(&mut buffer) {
                Ok(0) => return Ok(()),
                Err(error) => {
                    let _ = crate::proc::kill_process_group(pid);
                    return Err(error);
                }
                Ok(read) => read,
            };
            let mut bytes = self.lock();
            let remaining = self.maximum.saturating_sub(bytes.len());
            let accepted = read.min(remaining);
            bytes.extend_from_slice(&buffer[..accepted]);
            drop(bytes);
            if accepted != read && !self.overflowed.swap(true, Ordering::AcqRel) {
                let _ = crate::proc::kill_process_group(pid);
            }
        }
    }

    fn overflowed(&self) -> bool {
        self.overflowed.load(Ordering::Acquire)
    }

    fn take_bytes(&self) -> Vec<u8> {
        std::mem::take(&mut *self.lock())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<u8>> {
        self.bytes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}
