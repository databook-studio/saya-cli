//! The run event journal's contract: one append is exactly one NDJSON line,
//! nothing secret-shaped ever reaches the run dir's bytes, and a resume
//! reads back the state the journal recorded.

use std::{
    fs,
    path::{Path, PathBuf},
};

use saya_harness::journal::{Journal, StepState, replay};
use saya_types::{PauseReason, RunEvent, RunFailureCode};

const SENTINEL: &str = "sk-live-SENTINEL";

fn temp_run_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "saya-harness-journal-{label}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

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

#[test]
fn resume_reads_back_the_last_state() {
    let dir = temp_run_dir("resume");
    let journal = Journal::open(&dir);
    journal.append(&RunEvent::RunStarted).unwrap();
    journal
        .append(&RunEvent::PlanApproved { scopes: vec![] })
        .unwrap();
    journal.append(&RunEvent::StepStarted { step: 0 }).unwrap();
    journal
        .append(&RunEvent::StepCompleted { step: 0 })
        .unwrap();
    journal.append(&RunEvent::StepStarted { step: 1 }).unwrap();

    let state = journal.rebuild().unwrap();
    assert!(state.started);
    assert!(state.plan_approved);
    assert_eq!(state.steps.get(&0), Some(&StepState::Completed));
    assert_eq!(state.steps.get(&1), Some(&StepState::Started));
    assert_eq!(
        state.last.as_ref(),
        Some(&RunEvent::StepStarted { step: 1 })
    );

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn replay_records_a_bounded_retry_and_lets_usage_stand_aside() {
    let events = vec![
        RunEvent::RunStarted,
        RunEvent::PlanApproved { scopes: vec![] },
        RunEvent::StepStarted { step: 0 },
        RunEvent::StepFailed { step: 0 },
        RunEvent::StepStarted { step: 0 },
        RunEvent::Paused {
            reason: PauseReason::BudgetExhausted,
        },
        RunEvent::Usage {
            endpoint: "deepseek".to_string(),
            tokens: Some(1),
            turns: None,
            tool_calls: None,
            cached_input_tokens: None,
            cache_creation_input_tokens: None,
        },
    ];

    let state = replay(&events);
    // The retry's `StepStarted` wins over the earlier `StepFailed`.
    assert_eq!(state.steps.get(&0), Some(&StepState::Started));
    // A pause is the last lifecycle event; a usage report after it must not
    // stand in for one.
    assert_eq!(
        state.last,
        Some(RunEvent::Paused {
            reason: PauseReason::BudgetExhausted
        })
    );
}

#[test]
fn replay_records_terminal_states_and_their_failure_code() {
    let events = vec![
        RunEvent::RunStarted,
        RunEvent::PlanApproved { scopes: vec![] },
        RunEvent::StepStarted { step: 0 },
        RunEvent::Failed {
            code: RunFailureCode::SafetyQuery,
        },
    ];

    let state = replay(&events);
    assert_eq!(
        state.last,
        Some(RunEvent::Failed {
            code: RunFailureCode::SafetyQuery
        })
    );
}

#[test]
fn an_absent_journal_reads_back_empty() {
    let dir = temp_run_dir("absent");
    let journal = Journal::open(&dir);
    assert_eq!(journal.read().unwrap(), Vec::new());
    let state = journal.rebuild().unwrap();
    assert_eq!(state, Default::default());
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn a_torn_tail_left_by_a_crash_is_ignored_but_complete_lines_must_parse() {
    let dir = temp_run_dir("torn");
    let journal = Journal::open(&dir);
    journal.append(&RunEvent::RunStarted).unwrap();
    journal
        .append(&RunEvent::PlanApproved { scopes: vec![] })
        .unwrap();

    // Simulate a crash mid-append: a partial final line with no newline.
    let complete = fs::read_to_string(dir.join("events.ndjson")).unwrap();
    fs::write(
        dir.join("events.ndjson"),
        format!("{complete}{{\"type\":\"ste"),
    )
    .unwrap();

    let events = journal.read().unwrap();
    assert_eq!(
        events,
        vec![
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: vec![] }
        ],
        "a torn tail was never a complete event; it must not poison the read"
    );

    // A *complete* line that does not parse is corruption, refused.
    fs::write(
        dir.join("events.ndjson"),
        format!("{complete}{{\"type\":\"no-such-event\"}}\n"),
    )
    .unwrap();
    let error = journal.read().unwrap_err();
    assert!(
        matches!(
            error,
            saya_harness::HarnessError::JournalCorrupt { line: 3 }
        ),
        "expected JournalCorrupt at line 3, got {error:?}"
    );

    let _ = fs::remove_dir_all(dir);
}
