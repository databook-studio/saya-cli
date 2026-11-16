use std::{
    fs,
    io::{IsTerminal as _, Read},
    path::PathBuf,
    sync::mpsc,
    thread,
    time::Duration,
};

/// Ceiling on a prompt/SQL read from stdin. A prompt is prose and `--file` is
/// the path for anything large, so this stays in the hundreds of kilobytes —
/// generous for any hand-written prompt or a single piped SQL statement, but
/// small enough to fail fast when someone pipes a misdirected large file or a
/// whole dump. Exceeding it is an error naming the limit, never a truncation.
pub(super) const STDIN_BYTE_LIMIT: usize = 512 * 1024;
/// File-backed prompts and SQL use the same ceiling as piped input. Keeping
/// the bound here means `--file` cannot turn the otherwise bounded input path
/// into an unbounded allocation.
pub(crate) const FILE_BYTE_LIMIT: usize = STDIN_BYTE_LIMIT;

/// How long the stdin read may sit silent before we give up. The scripting path
/// (`echo x | saya ask`) delivers bytes in milliseconds; an idle pipe (CI,
/// systemd, a supervisor that left stdin open with no writer) never delivers
/// anything and would otherwise block `read` forever. This is an *inactivity*
/// deadline — reset on every byte — so a slow-but-steady producer is not killed.
///
/// It also governs the *first* byte, where it cannot distinguish an idle pipe
/// from a producer that is simply slow to start. The two costs are not
/// symmetric: waiting longer before reporting a genuine hang is a slower clear
/// error, while cutting off a real producer is a broken pipeline with a
/// misleading one. Hence a value well above any plausible start-up delay
/// rather than the tightest one that catches the hang.
const STDIN_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

/// Read up to `limit` bytes from `reader`, decoding as UTF-8. Returns the full
/// string at EOF; errors if the stream exceeds `limit`, is invalid UTF-8, or
/// the read fails. `on_progress` runs after every chunk so a caller can prove
/// liveness between reads.
fn read_bounded_progress<R: Read>(
    reader: &mut R,
    limit: usize,
    mut on_progress: impl FnMut(),
) -> Result<String, InputReadError> {
    let mut bytes: Vec<u8> = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = reader.read(&mut buf).map_err(InputReadError::Io)?;
        if n == 0 {
            break;
        }
        bytes.extend_from_slice(&buf[..n]);
        if bytes.len() > limit {
            return Err(InputReadError::OverLimit { limit });
        }
        on_progress();
    }
    String::from_utf8(bytes).map_err(InputReadError::Utf8)
}

/// Bounded read with no liveness callback — the shape tests use directly to
/// exercise the bound on a `Cursor` without threading.
fn read_bounded<R: Read>(reader: &mut R, limit: usize) -> Result<String, InputReadError> {
    read_bounded_progress(reader, limit, || {})
}

pub(crate) fn read_file_bounded(
    path: &std::path::Path,
    limit: usize,
) -> Result<String, InputReadError> {
    let mut file = fs::File::open(path).map_err(InputReadError::Io)?;
    read_bounded(&mut file, limit)
}

/// Read stdin on a worker thread, giving up if it stays silent past `idle`. A
/// blocking `read` on an idle pipe would hang the caller forever; off-thread
/// lets the inactivity deadline interrupt the wait (the parked read is abandoned
/// and dies with the process). `idle` is a parameter so tests run fast.
fn read_stdin_bounded_with_deadline<R: Read + Send + 'static>(
    reader: R,
    limit: usize,
    idle: Duration,
) -> Result<String, InputReadError> {
    enum Signal {
        Progress,
        Done(Result<String, InputReadError>),
    }
    let (tx, rx) = mpsc::channel::<Signal>();
    thread::spawn(move || {
        let mut reader = reader;
        let progress = || {
            let _ = tx.send(Signal::Progress);
        };
        let _ = tx.send(Signal::Done(read_bounded_progress(
            &mut reader,
            limit,
            progress,
        )));
    });
    loop {
        match rx.recv_timeout(idle) {
            Ok(Signal::Progress) => continue,
            Ok(Signal::Done(result)) => return result,
            Err(mpsc::RecvTimeoutError::Timeout) => return Err(InputReadError::Idle),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(InputReadError::Io(std::io::Error::other(
                    "stdin reader failed",
                )));
            }
        }
    }
}

/// Failure of a bounded/deadlined stdin read. `OverLimit` renders the limit so
/// the user is told the size refused; `Idle` says stdin was silent so the
/// caller learns the read did not hang — it gave up.
#[derive(Debug)]
pub(crate) enum InputReadError {
    OverLimit { limit: usize },
    Idle,
    Io(std::io::Error),
    Utf8(std::string::FromUtf8Error),
}

impl std::fmt::Display for InputReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OverLimit { limit } => write!(
                f,
                "input exceeds the {limit}-byte limit; use a smaller input"
            ),
            Self::Idle => write!(
                f,
                "no input arrived on stdin within {:?}; pass a prompt, use --file, or pipe data",
                STDIN_IDLE_TIMEOUT
            ),
            Self::Io(e) => write!(f, "reading stdin: {e}"),
            Self::Utf8(e) => write!(f, "stdin is not valid UTF-8: {e}"),
        }
    }
}

impl std::error::Error for InputReadError {}

pub(super) fn input(
    value: Option<String>,
    file: Option<PathBuf>,
) -> Result<String, Box<dyn std::error::Error>> {
    match (value, file) {
        (Some(value), None) => Ok(value),
        (None, Some(path)) => Ok(read_file_bounded(&path, FILE_BYTE_LIMIT)?),
        (Some(_), Some(_)) => Err("provide a prompt or --file, not both".into()),
        // Piped input (`pbpaste | saya ask`, `echo sql | saya query`) is the
        // scripting path: slurp stdin instead of demanding an argument. The read
        // is bounded and cannot hang on an idle pipe — see [`STDIN_BYTE_LIMIT`]
        // and [`STDIN_IDLE_TIMEOUT`].
        (None, None) if !std::io::stdin().is_terminal() => {
            let buffer = read_stdin_bounded_with_deadline(
                std::io::stdin(),
                STDIN_BYTE_LIMIT,
                STDIN_IDLE_TIMEOUT,
            )?;
            let trimmed = buffer.trim();
            if trimmed.is_empty() {
                Err("a prompt or --file is required".into())
            } else {
                Ok(trimmed.to_string())
            }
        }
        (None, None) => Err("a prompt or --file is required".into()),
    }
}

#[cfg(test)]
#[path = "query_input/tests.rs"]
mod tests;
