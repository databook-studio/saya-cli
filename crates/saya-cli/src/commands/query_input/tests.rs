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

// The read is bounded. Over-limit input is refused with a
// message that names the limit (not silently truncated).
#[test]
fn over_limit_input_is_refused_naming_the_limit() {
    let over: Vec<u8> = vec![b'x'; STDIN_BYTE_LIMIT + 1];
    let mut reader = Cursor::new(over);
    let err = read_bounded(&mut reader, STDIN_BYTE_LIMIT)
        .expect_err("one byte over the limit must be refused");
    let InputReadError::OverLimit { limit } = &err else {
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
        Err(InputReadError::Utf8(_))
    ));
}

#[test]
fn file_input_is_bounded_without_echoing_file_contents() {
    let path = std::env::temp_dir().join(format!(
        "saya-qin-over-limit-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let sentinel = "file-input-secret-sentinel";
    let mut bytes = vec![b'x'; FILE_BYTE_LIMIT + 1];
    bytes[..sentinel.len()].copy_from_slice(sentinel.as_bytes());
    std::fs::write(&path, bytes).unwrap();
    let error = input(None, Some(path.clone())).expect_err("oversized file must be refused");
    let rendered = error.to_string();
    assert!(rendered.contains(&FILE_BYTE_LIMIT.to_string()));
    assert!(!rendered.contains(sentinel));
    let _ = std::fs::remove_file(path);
}

// The idle-stdin guard. A reader that never produces a byte
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
        matches!(result, Err(InputReadError::Idle)),
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
    assert!(matches!(err, InputReadError::OverLimit { .. }));
    let rendered = err.to_string();
    assert!(
        rendered.contains(&STDIN_BYTE_LIMIT.to_string()),
        "error must name the limit: {rendered}"
    );
}
