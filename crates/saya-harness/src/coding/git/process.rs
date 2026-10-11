use std::{
    io::{Read, Write},
    path::Path,
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use saya_agent::CancellationToken;
use tokio::time::Instant;

use super::{
    command::GitError,
    environment::GitEnvironment,
    process_group::GitProcessGroup,
    process_observer::{GitProcessObserver, Observation},
    request::GitRequest,
};

const PUMP_CHUNK_BYTES: usize = 8 * 1024;

pub(super) struct GitOutput {
    pub(super) exit_code: Option<i32>,
    pub(super) signal: Option<i32>,
    pub(super) stdout: Vec<u8>,
}

pub(super) async fn execute(
    environment: &GitEnvironment,
    current_dir: &Path,
    request: GitRequest<'_>,
    cancellation: &CancellationToken,
    observer: &Arc<GitProcessObserver>,
) -> Result<GitOutput, GitError> {
    execute_inner(
        environment,
        current_dir,
        request,
        cancellation,
        None,
        observer,
    )
    .await
}

pub(super) async fn execute_until(
    environment: &GitEnvironment,
    current_dir: &Path,
    request: GitRequest<'_>,
    cancellation: &CancellationToken,
    deadline: Instant,
    observer: &Arc<GitProcessObserver>,
) -> Result<GitOutput, GitError> {
    execute_inner(
        environment,
        current_dir,
        request,
        cancellation,
        Some(deadline),
        observer,
    )
    .await
}

async fn execute_inner(
    environment: &GitEnvironment,
    current_dir: &Path,
    request: GitRequest<'_>,
    cancellation: &CancellationToken,
    deadline: Option<Instant>,
    observer: &Arc<GitProcessObserver>,
) -> Result<GitOutput, GitError> {
    let mut observation = observer.begin(request.kind(), request.operation());
    let result = execute_observed(
        environment,
        current_dir,
        request,
        cancellation,
        deadline,
        &mut observation,
    )
    .await;
    observation.finish(&result);
    result
}

async fn execute_observed(
    environment: &GitEnvironment,
    current_dir: &Path,
    request: GitRequest<'_>,
    cancellation: &CancellationToken,
    deadline: Option<Instant>,
    observation: &mut Observation,
) -> Result<GitOutput, GitError> {
    let operation = request.operation();
    let (stdout_limit, stderr_limit) = request.output_limits();
    let input = request.stdin_bytes()?;
    let mut command = Command::new(&environment.executable);
    environment.apply(&mut command);
    command
        .arg("--no-lazy-fetch")
        .args(request.arguments())
        .current_dir(current_dir)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        command.process_group(0);
    }
    if deadline.is_some_and(|deadline| deadline <= Instant::now()) {
        return Err(GitError::TimedOut);
    }
    let mut child = command.spawn().map_err(GitError::Spawn)?;
    let pid = child.id();
    observation.pid(pid);
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
    let input_writer = if let Some(input) = input {
        let Some(pipe) = child.stdin.take() else {
            let _ = crate::proc::kill_process_group(pid);
            let _ = child.wait();
            process_group.disarm();
            return Err(GitError::CaptureUnavailable);
        };
        Some(start_input(pipe, input))
    } else {
        None
    };

    let stdout = Arc::new(Capture::new(stdout_limit));
    let stderr = Arc::new(Capture::new(stderr_limit));
    let stdout_reader = start_pump(Arc::clone(&stdout), stdout_pipe, pid);
    let stderr_reader = start_pump(Arc::clone(&stderr), stderr_pipe, pid);
    let wait_timeout = deadline.map_or_else(
        || operation.timeout(),
        |deadline| {
            operation
                .timeout()
                .min(deadline.saturating_duration_since(Instant::now()))
        },
    );
    let settled = match crate::proc::wait(child, wait_timeout, cancellation).await {
        Ok(settled) => {
            process_group.disarm();
            settled
        }
        Err(error) => return Err(GitError::Wait(error)),
    };
    let stdout_result = stdout_reader.await;
    let stderr_result = stderr_reader.await;
    let input_result = if let Some(writer) = input_writer {
        Some(writer.await)
    } else {
        None
    };
    observation.settled(settled.duration_ms, settled.killed_orphans);
    observation.capture(
        stdout.total_bytes(),
        stderr.total_bytes(),
        stdout.overflowed(),
        stderr.overflowed(),
    );
    if !matches!(stdout_result, Ok(Ok(()))) || !matches!(stderr_result, Ok(Ok(()))) {
        return Err(GitError::CaptureUnavailable);
    }
    if stdout.overflowed() || stderr.overflowed() {
        return Err(GitError::OutputLimitExceeded);
    }
    if settled.end == crate::proc::WaitEnd::Exited {
        match input_result {
            None | Some(Ok(Ok(()))) => {}
            Some(Ok(Err(error))) => return Err(GitError::InputUnavailable(error)),
            Some(Err(_)) => return Err(GitError::CaptureUnavailable),
        }
    }
    match settled.end {
        crate::proc::WaitEnd::TimedOut => Err(GitError::TimedOut),
        crate::proc::WaitEnd::Cancelled => Err(GitError::Cancelled),
        crate::proc::WaitEnd::Exited => Ok(GitOutput {
            exit_code: settled.status.code(),
            signal: signal(&settled.status),
            stdout: stdout.take_bytes(),
        }),
    }
}

fn signal(status: &std::process::ExitStatus) -> Option<i32> {
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

fn start_input(
    mut writer: std::process::ChildStdin,
    input: Vec<u8>,
) -> tokio::task::JoinHandle<std::io::Result<()>> {
    tokio::task::spawn_blocking(move || writer.write_all(&input))
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
    total_bytes: AtomicU64,
}

impl Capture {
    fn new(maximum: usize) -> Self {
        Self {
            bytes: Mutex::new(Vec::new()),
            maximum,
            overflowed: AtomicBool::new(false),
            total_bytes: AtomicU64::new(0),
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
            self.total_bytes.fetch_add(read as u64, Ordering::AcqRel);
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

    fn total_bytes(&self) -> u64 {
        self.total_bytes.load(Ordering::Acquire)
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
