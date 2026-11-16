//! Unit tests for the progress classifier and repeat detector.

use std::time::Duration;

use serde_json::json;

use super::{
    DEFAULT_NO_PROGRESS_TURNS, DEFAULT_REPEAT_LIMIT, ProgressClassifier, ProgressLimits,
    ProgressStopKind, ToolObservation,
};

pub(super) fn limits() -> ProgressLimits {
    ProgressLimits::default()
}

pub(super) fn observe<'a>(
    classifier: &mut ProgressClassifier,
    name: &'a str,
    arguments: &'a serde_json::Value,
    result: &'a serde_json::Value,
) -> super::Observed {
    classifier.observe(ToolObservation {
        name,
        arguments,
        executed: true,
        succeeded: true,
        result,
    })
}

pub(super) fn observe_failed<'a>(
    classifier: &mut ProgressClassifier,
    name: &'a str,
    arguments: &'a serde_json::Value,
    result: &'a serde_json::Value,
) -> super::Observed {
    classifier.observe(ToolObservation {
        name,
        arguments,
        executed: true,
        succeeded: false,
        result,
    })
}

#[test]
fn successful_write_family_calls_are_progress() {
    let mut classifier = ProgressClassifier::new(limits());
    for (name, arguments) in [
        ("write", json!({"path": "a.rs", "content": "x"})),
        ("edit", json!({"path": "a.rs", "old": "x", "new": "y"})),
        ("delete", json!({"path": "b.rs"})),
        ("rewind", json!({"turn": 2})),
    ] {
        let observed = observe(&mut classifier, name, &arguments, &json!({"ok": true}));
        assert!(observed.progress, "{name} must count as progress");
    }
}

#[test]
fn failed_write_is_not_progress() {
    let mut classifier = ProgressClassifier::new(limits());
    let arguments = json!({"path": "a.rs", "content": "x"});
    let observed = observe_failed(
        &mut classifier,
        "write",
        &arguments,
        &json!({"error": "denied"}),
    );
    assert!(!observed.progress);
}

#[test]
fn a_denied_call_is_not_progress() {
    let mut classifier = ProgressClassifier::new(limits());
    let arguments = json!({"path": "a.rs", "content": "x"});
    let observed = classifier.observe(ToolObservation {
        name: "write",
        arguments: &arguments,
        executed: false,
        succeeded: false,
        result: &json!({"error": "denied"}),
    });
    assert!(!observed.progress);
}

#[test]
fn first_read_of_a_path_is_progress_and_a_reread_is_not() {
    let mut classifier = ProgressClassifier::new(limits());
    let arguments = json!({"path": "src/lib.rs"});
    let result = json!({"content": "same"});
    assert!(observe(&mut classifier, "read", &arguments, &result).progress);
    assert!(!observe(&mut classifier, "read", &arguments, &result).progress);
    assert!(!observe(&mut classifier, "read", &arguments, &result).progress);
}

#[test]
fn a_grep_that_finds_something_is_progress_once_and_a_rerun_is_not() {
    let mut classifier = ProgressClassifier::new(limits());
    let arguments = json!({"pattern": "TODO"});
    let result = json!({"matches": [{"path": "src/lib.rs", "line": 3}]});
    assert!(observe(&mut classifier, "grep", &arguments, &result).progress);
    assert!(!observe(&mut classifier, "grep", &arguments, &result).progress);
    let other = json!({"pattern": "FIXME"});
    assert!(observe(&mut classifier, "grep", &other, &result).progress);
}

#[test]
fn a_shell_command_is_progress_when_its_outcome_changes() {
    let mut classifier = ProgressClassifier::new(limits());
    let arguments = json!({"command": "cargo test"});
    let passing = json!({"exit_code": 0, "stdout": "ok", "stderr": ""});
    let failing = json!({"exit_code": 1, "stdout": "", "stderr": "error[E0308]"});
    assert!(observe(&mut classifier, "shell", &arguments, &passing).progress);
    // Identical outcome: the same command repeating is not progress.
    assert!(!observe(&mut classifier, "shell", &arguments, &passing).progress);
    // A changed outcome is.
    assert!(observe(&mut classifier, "shell", &arguments, &failing).progress);
    assert!(!observe(&mut classifier, "shell", &arguments, &failing).progress);
    // Whitespace alone does not read as a new outcome.
    let reflowed = json!({"exit_code": 1, "stdout": "", "stderr": "error[E0308] "});
    assert!(!observe(&mut classifier, "shell", &arguments, &reflowed).progress);
}

#[test]
fn changed_failing_output_is_progress_even_at_the_same_exit_code() {
    let mut classifier = ProgressClassifier::new(limits());
    let arguments = json!({"command": "cargo test"});
    let first = json!({"exit_code": 1, "stderr": "error[E0308]: mismatched types"});
    let second = json!({"exit_code": 1, "stderr": "error[E0425]: cannot find value"});
    assert!(observe(&mut classifier, "shell", &arguments, &first).progress);
    assert!(observe(&mut classifier, "shell", &arguments, &second).progress);
}

#[test]
fn checklist_change_and_plan_submission_are_progress() {
    let mut classifier = ProgressClassifier::new(limits());
    let arguments = json!({"items": []});
    let open = json!({"items": [{"text": "a", "status": "open"}]});
    let done = json!({"items": [{"text": "a", "status": "done"}]});
    assert!(observe(&mut classifier, "checklist", &arguments, &open).progress);
    assert!(!observe(&mut classifier, "checklist", &arguments, &open).progress);
    assert!(observe(&mut classifier, "checklist", &arguments, &done).progress);
    assert!(
        observe(
            &mut classifier,
            "submit_plan",
            &arguments,
            &json!({"ok": true})
        )
        .progress
    );
}

#[test]
fn unknown_tools_are_not_progress() {
    let mut classifier = ProgressClassifier::new(limits());
    let arguments = json!({});
    assert!(
        !observe(
            &mut classifier,
            "web_search",
            &arguments,
            &json!({"hits": 1})
        )
        .progress
    );
}

#[test]
fn text_only_turns_stop_after_the_default_no_progress_turns() {
    let mut classifier = ProgressClassifier::new(limits());
    for turn in 1..DEFAULT_NO_PROGRESS_TURNS {
        assert_eq!(classifier.finish_turn(None), None, "turn {turn}");
    }
    let stop = classifier
        .finish_turn(None)
        .expect("the eighth text-only turn stops");
    assert_eq!(stop.kind, ProgressStopKind::NoProgress);
    assert_eq!(stop.kind.class(), "no_progress");
    assert_eq!(stop.limit, "limits.job.no_progress_turns");
    assert_eq!(stop.limit_value, Some(u64::from(DEFAULT_NO_PROGRESS_TURNS)));
    assert_eq!(stop.measured, u64::from(DEFAULT_NO_PROGRESS_TURNS));
    assert!(stop.message.contains("no_progress_turns"));
}

#[test]
fn progress_resets_the_no_progress_counter() {
    let mut classifier = ProgressClassifier::new(limits());
    let arguments = json!({"path": "src/lib.rs"});
    let result = json!({"content": "x"});
    for _ in 0..7 {
        assert_eq!(classifier.finish_turn(None), None);
    }
    assert!(observe(&mut classifier, "read", &arguments, &result).progress);
    assert_eq!(classifier.finish_turn(None), None);
    for _ in 0..7 {
        assert_eq!(classifier.finish_turn(None), None);
    }
    let stop = classifier
        .finish_turn(None)
        .expect("eight more turn without progress");
    assert_eq!(stop.measured, u64::from(DEFAULT_NO_PROGRESS_TURNS));
}

#[test]
fn zero_no_progress_turns_disables_the_guard() {
    let mut classifier = ProgressClassifier::new(ProgressLimits {
        no_progress_turns: Some(0),
        ..limits()
    });
    for _ in 0..50 {
        assert_eq!(classifier.finish_turn(None), None);
    }
}

#[test]
fn repeat_limit_notes_once_then_stops() {
    let mut classifier = ProgressClassifier::new(limits());
    let arguments = json!({"path": "src/lib.rs"});
    let result = json!({"content": "same"});
    for _ in 0..(DEFAULT_REPEAT_LIMIT - 1) {
        assert!(
            observe(&mut classifier, "read", &arguments, &result)
                .note
                .is_none()
        );
    }
    let noted = observe(&mut classifier, "read", &arguments, &result);
    assert_eq!(
        noted.note.as_deref(),
        Some("You are repeating read with the same result; change approach or finish.")
    );
    assert!(noted.stop.is_none());
    let stopped = observe(&mut classifier, "read", &arguments, &result);
    let stop = stopped.stop.expect("the next identical result stops");
    assert_eq!(stop.kind, ProgressStopKind::Repeating);
    assert_eq!(stop.kind.class(), "repeating");
    assert_eq!(stop.limit, "limits.job.repeat_limit");
    assert_eq!(stop.limit_value, Some(u64::from(DEFAULT_REPEAT_LIMIT)));
    assert_eq!(stop.measured, u64::from(DEFAULT_REPEAT_LIMIT) + 1);
}

#[test]
fn a_changed_argument_or_result_restarts_the_repeat_count() {
    let mut classifier = ProgressClassifier::new(limits());
    let first = json!({"path": "a.rs"});
    let second = json!({"path": "b.rs"});
    let result = json!({"content": "same"});
    for _ in 0..DEFAULT_REPEAT_LIMIT {
        observe(&mut classifier, "read", &first, &result);
    }
    // Different path: a fresh call, so no stop and no carry-over note.
    let other = observe(&mut classifier, "read", &second, &result);
    assert!(other.note.is_none() && other.stop.is_none());
    // Same path, different result: also fresh.
    let changed = observe(
        &mut classifier,
        "read",
        &second,
        &json!({"content": "changed"}),
    );
    assert!(changed.note.is_none() && changed.stop.is_none());
}

#[test]
fn a_different_tool_with_the_same_arguments_is_not_a_repeat() {
    let mut classifier = ProgressClassifier::new(limits());
    let arguments = json!({"path": "src/lib.rs"});
    let result = json!({"content": "same"});
    for _ in 0..DEFAULT_REPEAT_LIMIT {
        observe(&mut classifier, "read", &arguments, &result);
    }
    let other = observe(&mut classifier, "grep", &arguments, &result);
    assert!(other.note.is_none() && other.stop.is_none());
}

#[test]
fn zero_repeat_limit_disables_the_detector() {
    let mut classifier = ProgressClassifier::new(ProgressLimits {
        repeat_limit: Some(0),
        no_progress_turns: Some(0),
        ..limits()
    });
    let arguments = json!({"path": "src/lib.rs"});
    let result = json!({"content": "same"});
    for _ in 0..10 {
        let observed = observe(&mut classifier, "read", &arguments, &result);
        assert!(observed.note.is_none() && observed.stop.is_none());
    }
}

#[test]
fn wall_clock_stops_as_a_hard_guard() {
    let mut classifier = ProgressClassifier::new(ProgressLimits {
        wall_clock: Some(Duration::ZERO),
        ..limits()
    });
    let stop = classifier.finish_turn(None).expect("the clock guard fires");
    assert_eq!(stop.kind, ProgressStopKind::WallClock);
    assert_eq!(stop.kind.class(), "wall_clock");
    assert_eq!(stop.limit, "limits.job.wall_clock_seconds");
    assert!(!stop.kind.is_progress());
}

#[test]
fn cost_stops_when_the_reported_total_passes_the_ceiling() {
    let mut classifier = ProgressClassifier::new(ProgressLimits {
        max_cost_micros: Some(1_000),
        ..limits()
    });
    assert_eq!(classifier.finish_turn(Some(900)), None);
    let stop = classifier
        .finish_turn(Some(1_001))
        .expect("cost guard fires");
    assert_eq!(stop.kind, ProgressStopKind::Cost);
    assert_eq!(stop.kind.class(), "cost");
    assert_eq!(stop.limit, "limits.job.max_cost_usd");
    assert_eq!(stop.limit_value, Some(1_000));
    assert_eq!(stop.measured, 1_001);
}

#[test]
fn the_report_counts_turns_progress_and_events() {
    let mut classifier = ProgressClassifier::new(limits());
    let arguments = json!({"path": "src/lib.rs"});
    let result = json!({"content": "x"});
    observe(&mut classifier, "read", &arguments, &result);
    classifier.finish_turn(None);
    classifier.finish_turn(None);
    let report = classifier.report();
    assert_eq!(report.turns, 2);
    assert_eq!(report.last_progress_turn, Some(1));
    assert_eq!(report.progress_events.get("new_read"), Some(&1));
}

#[test]
fn progress_stops_are_the_clean_ones() {
    assert!(ProgressStopKind::NoProgress.is_progress());
    assert!(ProgressStopKind::Repeating.is_progress());
    assert!(!ProgressStopKind::WallClock.is_progress());
    assert!(!ProgressStopKind::Cost.is_progress());
}
