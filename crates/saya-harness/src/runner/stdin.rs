use std::{
    io::{self, Write},
    process::ChildStdin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use super::stdin_channel::StdinInputPipe;

const WRITER_POLL: Duration = Duration::from_millis(2);

#[cfg(test)]
mod tests {
    use crate::runner::stdin_channel::{StdinSendError, input_channel};
    use saya_agent::CancellationToken;

    use super::StdinDelivery;

    #[test]
    fn setup_failure_closes_queue_with_exact_accepted_count() {
        let (sender, pipe) = input_channel(CancellationToken::new());
        assert!(matches!(sender.try_send(b"accepted"), Ok(8)));

        let failure = StdinDelivery::failed(&pipe, std::io::Error::other("setup failed"));

        assert_eq!(failure.accepted_bytes, 8);
        assert!(matches!(
            sender.try_send(b"racing input"),
            Err(StdinSendError::Closed)
        ));
    }
}

pub(crate) struct StdinDelivery {
    pub(crate) accepted_bytes: usize,
    pub(crate) written_bytes: usize,
    pub(crate) source: Option<io::Error>,
}

impl StdinDelivery {
    pub(crate) fn is_complete(&self) -> bool {
        self.source.is_none() && self.accepted_bytes == self.written_bytes
    }

    pub(crate) fn failed(pipe: &StdinInputPipe, source: io::Error) -> Self {
        Self {
            accepted_bytes: pipe.close_and_accepted_bytes(),
            written_bytes: 0,
            source: Some(source),
        }
    }
}

pub(crate) struct StdinWriter {
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<WriterResult>>,
    pipe: StdinInputPipe,
}

struct WriterResult {
    written_bytes: usize,
    source: Option<io::Error>,
}

impl StdinWriter {
    pub(crate) fn start(
        mut stdin: ChildStdin,
        pipe: StdinInputPipe,
    ) -> Result<Self, StdinDelivery> {
        if let Err(source) = set_nonblocking(&stdin) {
            return Err(StdinDelivery::failed(&pipe, source));
        }
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let worker_pipe = pipe.clone_for_writer();
        let thread = thread::Builder::new()
            .name("saya-command-stdin".into())
            .spawn(move || pump(&mut stdin, &worker_pipe, &thread_stop))
            .map_err(|source| StdinDelivery::failed(&pipe, source))?;
        Ok(Self {
            stop,
            thread: Some(thread),
            pipe,
        })
    }

    pub(crate) fn finish(mut self) -> StdinDelivery {
        self.stop.store(true, Ordering::Release);
        self.pipe.stop();
        self.join()
    }

    fn join(&mut self) -> StdinDelivery {
        let result = self.thread.take().and_then(|thread| thread.join().ok());
        let accepted_bytes = self.pipe.accepted_bytes();
        match result {
            Some(result) => StdinDelivery {
                accepted_bytes,
                written_bytes: result.written_bytes,
                source: result.source,
            },
            None => StdinDelivery {
                accepted_bytes,
                written_bytes: 0,
                source: Some(io::Error::other("stdin writer thread did not join")),
            },
        }
    }
}

impl Drop for StdinWriter {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.pipe.stop();
        let _ = self.join();
    }
}

fn pump(stdin: &mut ChildStdin, pipe: &StdinInputPipe, stop: &AtomicBool) -> WriterResult {
    let mut written_bytes = 0usize;
    loop {
        if stop.load(Ordering::Acquire) {
            return WriterResult {
                written_bytes,
                source: None,
            };
        }
        let Some(chunk) = pipe.next_chunk(stop, WRITER_POLL) else {
            return WriterResult {
                written_bytes,
                source: None,
            };
        };
        let mut offset = 0usize;
        while offset < chunk.len() {
            if stop.load(Ordering::Acquire) {
                return WriterResult {
                    written_bytes,
                    source: None,
                };
            }
            match stdin.write(&chunk[offset..]) {
                Ok(0) => {
                    return WriterResult {
                        written_bytes,
                        source: Some(io::Error::from(io::ErrorKind::WriteZero)),
                    };
                }
                Ok(written) => {
                    offset += written;
                    written_bytes = written_bytes.saturating_add(written);
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(WRITER_POLL);
                }
                Err(source) => {
                    return WriterResult {
                        written_bytes,
                        source: Some(source),
                    };
                }
            }
        }
    }
}

#[cfg(unix)]
fn set_nonblocking(stdin: &ChildStdin) -> io::Result<()> {
    use std::os::fd::AsRawFd as _;

    let fd = stdin.as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(unix))]
fn set_nonblocking(_stdin: &ChildStdin) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "post-spawn command stdin requires Unix nonblocking pipes",
    ))
}
