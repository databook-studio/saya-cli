//! Background-poll progress rules: the wait on a live run is not a stuck
//! turn, while a finished handle polled again is still caught. The wait is
//! bounded: past [`SILENT_POLL_LIMIT`] silent polls of one run the poll is
//! still a wait, up to the job's wall clock (or the fixed total wait bound
//! when the job has none); only past that bound does it count as an idle turn.

use std::time::Duration;

use serde_json::json;

use super::tests::{limits, observe, observe_failed};
use super::{
    DEFAULT_REPEAT_LIMIT, ProgressClassifier, ProgressLimits, ProgressStopKind, SILENT_POLL_LIMIT,
};

/// A model waiting on a silent build polls `shell_output` many times. Each
/// poll is unchanged, and each is a turn: up to the cap neither the repeat
/// detector nor the no-progress guard may read that as a stuck job.
#[test]
fn silent_polls_up_to_the_cap_are_neither_progress_nor_a_repeat() {
    let mut classifier = ProgressClassifier::new(limits());
    let arguments = json!({"id": 1});
    let result = json!({"id": 1, "status": "running", "stdout": "", "stderr": ""});
    for turn in 1..=SILENT_POLL_LIMIT {
        let observed = observe(&mut classifier, "shell_output", &arguments, &result);
        assert!(
            !observed.progress,
            "an unchanged poll is not progress (turn {turn})"
        );
        assert!(
            observed.note.is_none() && observed.stop.is_none(),
            "an unchanged poll is not a repeat (turn {turn}): {observed:?}"
        );
        assert_eq!(
            classifier.finish_turn(None),
            None,
            "turn {turn} of waiting on a live run must not trip a guard"
        );
    }
    // The run finishes, the model writes the fix, and the job ends: no guard
    // fired anywhere along the wait.
    let written = json!({"path": "src/lib.rs", "content": "x"});
    assert!(observe(&mut classifier, "write", &written, &json!({"digest": "d"})).progress);
    assert_eq!(classifier.finish_turn(None), None);
    assert_eq!(classifier.report().turns, SILENT_POLL_LIMIT as u64 + 1);
}

/// A silent build is waited on, not stopped: past the cap every silent poll
/// of a live run is still neutral, so the turn-level guard never sees it. The
/// run finishes, the model writes, and no guard fired along the way.
#[test]
fn a_silent_make_like_run_is_waited_to_completion_without_a_guard_stop() {
    let mut classifier = ProgressClassifier::new(limits());
    let arguments = json!({"id": 1});
    let quiet = json!({"id": 1, "status": "running", "stdout": "", "stderr": ""});
    for poll in 1..=20 {
        let observed = observe(&mut classifier, "shell_output", &arguments, &quiet);
        assert!(!observed.progress, "a silent poll is not progress ({poll})");
        assert!(
            observed.note.is_none() && observed.stop.is_none(),
            "a silent wait is not a repeat ({poll}): {observed:?}"
        );
        assert_eq!(
            classifier.finish_turn(None),
            None,
            "silent poll {poll} must not trip a guard"
        );
    }
    assert!(
        classifier.report().check.is_none(),
        "a neutral wait records no no-progress check"
    );
    let exited = json!({"id": 1, "status": "exited", "exit_code": 0, "stdout": "", "stderr": ""});
    assert!(observe(&mut classifier, "shell_output", &arguments, &exited).progress);
    let written = json!({"path": "src/lib.rs", "content": "x"});
    assert!(observe(&mut classifier, "write", &written, &json!({"digest": "d"})).progress);
    assert_eq!(classifier.finish_turn(None), None);
}

/// The wait is bounded: once the job's wall clock is spent, a silent poll is
/// an idle turn again. (The wall clock itself stops the job at the same turn
/// boundary; this asserts the classification that fed it.)
#[test]
fn silent_polls_are_idle_once_the_job_wall_clock_is_spent() {
    let mut classifier = ProgressClassifier::new(ProgressLimits {
        wall_clock: Some(Duration::from_millis(500)),
        ..limits()
    });
    let arguments = json!({"id": 1});
    let quiet = json!({"id": 1, "status": "running", "stdout": "", "stderr": ""});
    for _ in 0..SILENT_POLL_LIMIT {
        observe(&mut classifier, "shell_output", &arguments, &quiet);
        assert_eq!(classifier.finish_turn(None), None);
    }
    let observed = observe(&mut classifier, "shell_output", &arguments, &quiet);
    assert!(!observed.progress);
    assert_eq!(
        classifier.finish_turn(None),
        None,
        "inside the wall clock a silent poll is still a wait"
    );
    assert!(
        classifier.report().check.is_none(),
        "the neutral poll recorded no check"
    );

    std::thread::sleep(Duration::from_millis(600));
    let observed = observe(&mut classifier, "shell_output", &arguments, &quiet);
    assert!(!observed.progress);
    let stop = classifier
        .finish_turn(None)
        .expect("the wall clock guard fires at the turn boundary");
    assert_eq!(stop.kind, ProgressStopKind::WallClock);
    let check = classifier
        .report()
        .check
        .expect("past the wall clock a silent poll is an idle turn");
    assert_eq!(check.turns_without_progress, 1);
    assert!(!check.reset);
}

/// The count is per run, and new output resets it: a poll that saw the build
/// move is not part of the next wait's silent streak.
#[test]
fn new_output_resets_the_silent_poll_count() {
    let mut classifier = ProgressClassifier::new(limits());
    let arguments = json!({"id": 1});
    let quiet = json!({"id": 1, "status": "running", "stdout": "", "stderr": ""});
    for _ in 0..SILENT_POLL_LIMIT {
        observe(&mut classifier, "shell_output", &arguments, &quiet);
        assert_eq!(classifier.finish_turn(None), None);
    }
    let output = json!({"id": 1, "status": "running", "stdout": "compiling\n", "stderr": ""});
    assert!(observe(&mut classifier, "shell_output", &arguments, &output).progress);
    assert_eq!(classifier.finish_turn(None), None);
    // The cap starts again: the next silent polls are waits, not idle turns.
    for _ in 0..SILENT_POLL_LIMIT {
        observe(&mut classifier, "shell_output", &arguments, &quiet);
        assert_eq!(classifier.finish_turn(None), None);
    }
}

/// A handle that is finished (or dead) and polled again with no change is a
/// stuck loop, and the repeat guard still catches it.
#[test]
fn repeated_polls_of_a_finished_handle_are_still_caught() {
    let mut classifier = ProgressClassifier::new(limits());
    let arguments = json!({"id": 7});
    let result = json!({"error": "shell refused: this shell holds no background run with id 7"});
    let mut stop = None;
    for _ in 0..=DEFAULT_REPEAT_LIMIT {
        stop = observe_failed(&mut classifier, "shell_output", &arguments, &result).stop;
        if stop.is_some() {
            break;
        }
    }
    let stop = stop.expect("a finished handle polled with no change is still caught");
    assert_eq!(stop.kind, ProgressStopKind::Repeating);
}

/// A poll that returns new output, or a status the run has not reported
/// before, is progress: the build is moving.
#[test]
fn a_background_poll_is_progress_when_output_or_status_changes() {
    let mut classifier = ProgressClassifier::new(limits());
    let arguments = json!({"id": 1});
    let quiet = json!({"id": 1, "status": "running", "stdout": "", "stderr": ""});
    assert!(!observe(&mut classifier, "shell_output", &arguments, &quiet).progress);
    let output = json!({"id": 1, "status": "running", "stdout": "compiling\n", "stderr": ""});
    assert!(observe(&mut classifier, "shell_output", &arguments, &output).progress);
    let exited = json!({"id": 1, "status": "exited", "exit_code": 0, "stdout": "", "stderr": ""});
    assert!(observe(&mut classifier, "shell_output", &arguments, &exited).progress);
}
