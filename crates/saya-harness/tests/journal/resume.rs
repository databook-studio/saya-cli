//! Reading a journal back: a resume rebuilds the recorded state, an absent
//! journal reads empty, and a torn tail left by a crash is ignored while a
//! complete corrupt line is refused.

use std::fs;

use saya_harness::journal::{Journal, StepState};
use saya_types::RunEvent;

use super::temp_run_dir;

#[test]
fn resume_reads_back_the_last_state() {
    let dir = temp_run_dir("resume");
    let journal = Journal::open(&dir);
    journal.append(&RunEvent::RunStarted).unwrap();
    journal
        .append(&RunEvent::PlanApproved { scopes: None })
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
        .append(&RunEvent::PlanApproved { scopes: None })
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
            RunEvent::PlanApproved { scopes: None }
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
