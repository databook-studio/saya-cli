//! The per-turn no-progress check: silent on a healthy run, a count against
//! the limit on an idle turn, one reset line when progress resumes, and a stop
//! that names what was measured and how to continue.

use serde_json::json;

use super::{ProgressCheck, ProgressClassifier, ProgressLimits, ToolObservation};

/// A successful first read of `path`: progress, so the turn counts as healthy.
fn read(classifier: &mut ProgressClassifier, path: &str) -> bool {
    let arguments = json!({ "path": path });
    let result = json!({ "content": path });
    classifier
        .observe(ToolObservation {
            name: "read",
            arguments: &arguments,
            executed: true,
            succeeded: true,
            result: &result,
        })
        .progress
}

fn check(
    turn: u64,
    turns_without_progress: u32,
    limit: u32,
    reset: bool,
    last_progress_turn: Option<u64>,
) -> ProgressCheck {
    ProgressCheck {
        turn,
        turns_without_progress,
        limit: Some(limit),
        reset,
        last_progress_turn,
    }
}

#[test]
fn a_healthy_turn_is_silent_then_idle_turns_count_and_progress_resets_once() {
    let mut classifier = ProgressClassifier::new(ProgressLimits::default());

    // Turn 1: progress with a zero count — no noise on a healthy run.
    assert!(read(&mut classifier, "a.rs"));
    assert_eq!(classifier.finish_turn(None), None);
    assert_eq!(classifier.report().check, None);

    // Turns 2 and 3: no progress, recorded against the limit.
    assert_eq!(classifier.finish_turn(None), None);
    assert_eq!(
        classifier.report().check,
        Some(check(2, 1, 8, false, Some(1)))
    );
    assert_eq!(
        classifier.report().check.map(|check| check.message()),
        Some("no progress for 1 of 8 turns".to_owned())
    );

    assert_eq!(classifier.finish_turn(None), None);
    assert_eq!(
        classifier.report().check,
        Some(check(3, 2, 8, false, Some(1)))
    );

    // Turn 4: progress clears the streak, recorded once.
    assert!(read(&mut classifier, "b.rs"));
    assert_eq!(classifier.finish_turn(None), None);
    assert_eq!(
        classifier.report().check,
        Some(check(4, 2, 8, true, Some(4)))
    );
    assert_eq!(
        classifier.report().check.map(|check| check.message()),
        Some("progress resumed at turn 4; no-progress count reset (was 2)".to_owned())
    );

    // Turn 5: progress with a zero count is silent again.
    assert!(read(&mut classifier, "c.rs"));
    assert_eq!(classifier.finish_turn(None), None);
    assert_eq!(classifier.report().check, None);
}

#[test]
fn the_no_progress_stop_names_the_limit_turns_last_progress_and_how_to_continue() {
    let mut classifier = ProgressClassifier::new(ProgressLimits::default());
    assert!(read(&mut classifier, "a.rs"));
    classifier.finish_turn(None);

    let stop = (0..8)
        .find_map(|_| classifier.finish_turn(None))
        .expect("eight idle turns stop the job");
    assert!(
        stop.message.contains("no progress for 8 of 8 turns"),
        "{}",
        stop.message
    );
    assert!(
        stop.message.contains("last progress at turn 1"),
        "{}",
        stop.message
    );
    assert!(
        stop.message.contains("saya job resume <job>"),
        "{}",
        stop.message
    );
    assert!(
        stop.message.contains("limits.job.no_progress_turns"),
        "{}",
        stop.message
    );
}

#[test]
fn a_disabled_guard_reports_no_check() {
    let mut classifier = ProgressClassifier::new(ProgressLimits {
        no_progress_turns: Some(0),
        ..ProgressLimits::default()
    });
    for _ in 0..10 {
        assert_eq!(classifier.finish_turn(None), None);
    }
    assert_eq!(classifier.report().check, None);
}
