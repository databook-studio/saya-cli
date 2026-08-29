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
) -> Result<String, StdinReadError> {
    let mut bytes: Vec<u8> = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = reader.read(&mut buf).map_err(StdinReadError::Io)?;
        if n == 0 {
            break;
        }
        bytes.extend_from_slice(&buf[..n]);
        if bytes.len() > limit {
            return Err(StdinReadError::OverLimit { limit });
        }
        on_progress();
    }
    String::from_utf8(bytes).map_err(StdinReadError::Utf8)
}

/// Bounded read with no liveness callback — the shape tests use directly to
/// exercise the bound on a `Cursor` without threading.
#[cfg(test)]
fn read_bounded<R: Read>(reader: &mut R, limit: usize) -> Result<String, StdinReadError> {
    read_bounded_progress(reader, limit, || {})
}

/// Read stdin on a worker thread, giving up if it stays silent past `idle`. A
/// blocking `read` on an idle pipe would hang the caller forever; off-thread
/// lets the inactivity deadline interrupt the wait (the parked read is abandoned
/// and dies with the process). `idle` is a parameter so tests run fast.
fn read_stdin_bounded_with_deadline<R: Read + Send + 'static>(
    reader: R,
    limit: usize,
    idle: Duration,
) -> Result<String, StdinReadError> {
    enum Signal {
        Progress,
        Done(Result<String, StdinReadError>),
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
            Err(mpsc::RecvTimeoutError::Timeout) => return Err(StdinReadError::Idle),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(StdinReadError::Io(std::io::Error::other(
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
enum StdinReadError {
    OverLimit { limit: usize },
    Idle,
    Io(std::io::Error),
    Utf8(std::string::FromUtf8Error),
}

impl std::fmt::Display for StdinReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OverLimit { limit } => write!(
                f,
                "stdin input exceeds the {limit}-byte limit; use --file for larger input"
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

impl std::error::Error for StdinReadError {}

pub(super) fn input(
    value: Option<String>,
    file: Option<PathBuf>,
) -> Result<String, Box<dyn std::error::Error>> {
    match (value, file) {
        (Some(value), None) => Ok(value),
        (None, Some(path)) => Ok(fs::read_to_string(path)?),
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
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn explicit_inputs_win_and_conflicts_error() {
        assert_eq!(input(Some("hi".into()), None).unwrap(), "hi");
        let path = std::env::temp_dir().join(format!("saya-qin-{}.txt", std::process::id()));
        std::fs::write(&path, "from file").unwrap();
        assert_eq!(input(None, Some(path.clone())).unwrap(), "from file");
        let _ = std::fs::remove_file(&path);
        assert!(input(Some("a".into()), Some("b".into())).is_err());
        // In test harnesses stdin is non-TTY (pipe/null); empty stdin must
        // still demand an argument rather than submitting "".
    }

    // Deliverable 1: the read is bounded. Over-limit input is refused with a
    // message that names the limit (not silently truncated).
    #[test]
    fn over_limit_input_is_refused_naming_the_limit() {
        let over: Vec<u8> = vec![b'x'; STDIN_BYTE_LIMIT + 1];
        let mut reader = Cursor::new(over);
        let err = read_bounded(&mut reader, STDIN_BYTE_LIMIT)
            .expect_err("one byte over the limit must be refused");
        let StdinReadError::OverLimit { limit } = &err else {
            panic!("expected OverLimit, got {err:?}");
        };
        assert_eq!(*limit, STDIN_BYTE_LIMIT);
        let rendered = err.to_string();
        assert!(
            rendered.contains(&STDIN_BYTE_LIMIT.to_string()),
            "error must name the limit in bytes: {rendered}"
        );
    }

    // At-limit input is accepted — the boundary is inclusive of the limit, and
    // only *exceeding* it is refused. A silent off-by-one here would either
    // reject valid input or let one-too-many bytes through.
    #[test]
    fn at_limit_input_is_accepted() {
        let exactly: Vec<u8> = vec![b'y'; STDIN_BYTE_LIMIT];
        let mut reader = Cursor::new(exactly);
        let got = read_bounded(&mut reader, STDIN_BYTE_LIMIT).expect("at-limit is allowed");
        assert_eq!(got.len(), STDIN_BYTE_LIMIT);
    }

    // Empty input reads as EOF immediately (the `/dev/null` path relies on
    // this returning "" rather than blocking or erroring).
    #[test]
    fn empty_input_is_eof() {
        let mut reader = Cursor::new(Vec::<u8>::new());
        assert_eq!(read_bounded(&mut reader, STDIN_BYTE_LIMIT).unwrap(), "");
    }

    // The bound is on bytes; a multibyte sequence split across read calls must
    // still decode (we accumulate bytes and convert once at EOF, not per chunk).
    #[test]
    fn multibyte_utf8_split_across_reads_decodes() {
        // "é" is two bytes; feed it as one byte at a time.
        let bytes = "café".as_bytes().to_vec();
        let mut reader = Cursor::new(bytes);
        let got = read_bounded(&mut reader, STDIN_BYTE_LIMIT).unwrap();
        assert_eq!(got, "café");
    }

    // Invalid UTF-8 is an error (as read_to_string would have given), not a
    // silent acceptance of garbage.
    #[test]
    fn invalid_utf8_is_an_error() {
        let mut reader = Cursor::new(vec![0xff, 0xfe, 0xfd]);
        assert!(matches!(
            read_bounded(&mut reader, STDIN_BYTE_LIMIT),
            Err(StdinReadError::Utf8(_))
        ));
    }

    // Deliverable 3: the idle-stdin guard. A reader that never produces a byte
    // must give up within the idle deadline and report that stdin was silent —
    // not block forever. The deadline is injected so the test stays fast.
    #[test]
    fn silent_reader_gives_up_within_idle_deadline() {
        struct Silent;
        impl Read for Silent {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                std::thread::park(); // never returns, never sends a heartbeat
                Ok(0)
            }
        }
        let started = std::time::Instant::now();
        let result =
            read_stdin_bounded_with_deadline(Silent, STDIN_BYTE_LIMIT, Duration::from_millis(80));
        let elapsed = started.elapsed();
        assert!(
            matches!(result, Err(StdinReadError::Idle)),
            "expected Idle, got {result:?}"
        );
        // It gave up promptly — well under the production 10s, and not instant
        // (it actually waited for the deadline).
        assert!(elapsed >= Duration::from_millis(70));
        assert!(elapsed < Duration::from_secs(2));
    }

    // A reader that produces data within each idle window must succeed even
    // though it is slow overall — the deadline is *inactivity*, not total, so a
    // slow-but-steady producer is not killed.
    #[test]
    fn slow_but_steady_reader_is_not_killed() {
        // Two slow chunks then EOF: progress within each 200ms window.
        struct TwoSlowChunks(usize);
        impl Read for TwoSlowChunks {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                self.0 += 1;
                if self.0 > 2 {
                    return Ok(0);
                }
                std::thread::sleep(Duration::from_millis(30));
                buf[0] = b'k';
                Ok(1)
            }
        }
        let result = read_stdin_bounded_with_deadline(
            TwoSlowChunks(0),
            STDIN_BYTE_LIMIT,
            Duration::from_millis(200),
        );
        assert_eq!(result.unwrap(), "kk");
    }

    // The over-limit refusal still fires through the deadlined path.
    #[test]
    fn deadlined_path_refuses_over_limit() {
        let over = Cursor::new(vec![b'x'; STDIN_BYTE_LIMIT + 5]);
        let err = read_stdin_bounded_with_deadline(over, STDIN_BYTE_LIMIT, Duration::from_secs(5))
            .expect_err("over-limit must be refused");
        assert!(matches!(err, StdinReadError::OverLimit { .. }));
        let rendered = err.to_string();
        assert!(
            rendered.contains(&STDIN_BYTE_LIMIT.to_string()),
            "error must name the limit: {rendered}"
        );
    }
}
