//! The append surface: every append lands as exactly one newline-terminated
//! NDJSON line, and an event carrying a planted secret leaves no
//! secret-shaped bytes anywhere in the run dir.

use std::{fs, path::Path};

use saya_harness::journal::Journal;
use saya_types::RunEvent;

use super::temp_run_dir;

const SENTINEL: &str = "sk-live-SENTINEL";

#[track_caller]
fn assert_no_sentinel_bytes(dir: &Path, sentinel: &str) {
    fn walk(dir: &Path, sentinel: &str) {
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                walk(&path, sentinel);
            } else {
                let bytes = fs::read(&path).unwrap();
                let found = window_contains(&bytes, sentinel.as_bytes());
                assert!(
                    !found,
                    "LEAK: sentinel `{sentinel}` found in {} bytes",
                    path.display()
                );
            }
        }
    }
    walk(dir, sentinel);
}

/// Cross-boundary-safe substring search: a sentinel could straddle a read
/// buffer edge, so scan every window of the sentinel's length.
fn window_contains(bytes: &[u8], needle: &[u8]) -> bool {
    bytes.windows(needle.len()).any(|window| window == needle)
}

#[test]
fn append_writes_exactly_one_ndjson_line_per_event() {
    let dir = temp_run_dir("one-line");
    let journal = Journal::open(&dir);

    journal.append(&RunEvent::RunStarted).unwrap();
    let raw = fs::read_to_string(dir.join("events.ndjson")).unwrap();
    assert_eq!(raw.lines().count(), 1, "first append wrote: {raw:?}");
    assert!(raw.ends_with('\n'), "a line is newline-terminated: {raw:?}");

    journal.append(&RunEvent::StepStarted { step: 2 }).unwrap();
    let raw = fs::read_to_string(dir.join("events.ndjson")).unwrap();
    assert_eq!(raw.lines().count(), 2, "second append wrote: {raw:?}");

    // Every line parses back to the event that was appended, in order.
    assert_eq!(
        journal.read().unwrap(),
        vec![RunEvent::RunStarted, RunEvent::StepStarted { step: 2 }]
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(dir.join("events.ndjson"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "the journal is created 0600, got {mode:o}");
    }
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn an_event_carrying_a_planted_secret_writes_no_secret_to_the_run_dir() {
    let dir = temp_run_dir("sentinel");
    let journal = Journal::open(&dir);

    journal
        .append(&RunEvent::Usage {
            endpoint: format!("api_key={SENTINEL}"),
            tokens: Some(5),
            turns: None,
            tool_calls: None,
            cached_input_tokens: None,
            cache_creation_input_tokens: None,
        })
        .unwrap();

    // Byte-scan every file in the run dir — the knowledge-security recipe.
    assert_no_sentinel_bytes(&dir, SENTINEL);

    // Redaction runs inside the payload: the line is still parseable NDJSON
    // carrying the redaction marker, not a silently dropped event.
    let events = journal.read().unwrap();
    assert_eq!(events.len(), 1, "the event must survive, redacted");
    let RunEvent::Usage { endpoint, .. } = &events[0] else {
        panic!("unexpected event {:?}", events[0]);
    };
    assert_eq!(endpoint, "api_key=[redacted]");

    let _ = fs::remove_dir_all(dir);
}
