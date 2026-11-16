//! Material-progress rules and the call-level churn guard: a successful call
//! that moved nothing is idle, a real change resets the streak, and repeat
//! keys ignore incidental arguments.

use serde_json::json;

use super::tests::{limits, observe};
use super::{
    DEFAULT_NO_PROGRESS_TURNS, DEFAULT_REPEAT_LIMIT, ProgressClassifier, ProgressStopKind,
};

fn shell(exit: i64, stdout: &str) -> serde_json::Value {
    json!({"exit_code": exit, "stdout": stdout, "stderr": ""})
}

#[test]
fn a_new_shell_command_with_no_output_is_not_progress() {
    let mut classifier = ProgressClassifier::new(limits());
    let quiet = json!({"command": "true"});
    assert!(!observe(&mut classifier, "shell", &quiet, &shell(0, "")).progress);
    // A failure is information even when the command wrote nothing.
    let failing = json!({"command": "false"});
    assert!(observe(&mut classifier, "shell", &failing, &shell(1, "")).progress);
}

#[test]
fn a_shell_command_is_progress_only_when_its_output_changes() {
    let mut classifier = ProgressClassifier::new(limits());
    let arguments = json!({"command": "git status --short"});
    assert!(observe(&mut classifier, "shell", &arguments, &shell(0, " M a.rs\n")).progress);
    assert!(!observe(&mut classifier, "shell", &arguments, &shell(0, " M a.rs\n")).progress);
    assert!(
        observe(
            &mut classifier,
            "shell",
            &arguments,
            &shell(0, " M a.rs\n M b.rs\n")
        )
        .progress
    );
}

#[test]
fn whitespace_and_incidental_arguments_do_not_make_a_new_shell_command() {
    let mut classifier = ProgressClassifier::new(limits());
    let first = json!({"command": "echo  hi", "description": "one"});
    let second = json!({"command": "echo hi", "description": "two", "timeout": 5000});
    assert!(observe(&mut classifier, "shell", &first, &shell(0, "hi\n")).progress);
    assert!(!observe(&mut classifier, "shell", &second, &shell(0, "hi\n")).progress);
}

#[test]
fn a_search_with_no_matches_is_not_progress() {
    let mut classifier = ProgressClassifier::new(limits());
    let empty = json!({"matches": []});
    let found = json!({"matches": [{"path": "a.rs", "line": 1}]});
    let arguments = json!({"pattern": "TODO", "path": "src"});
    assert!(!observe(&mut classifier, "grep", &arguments, &empty).progress);
    assert!(observe(&mut classifier, "grep", &arguments, &found).progress);
    assert!(!observe(&mut classifier, "grep", &arguments, &found).progress);
    // A different pattern that still finds nothing is still not progress.
    let other = json!({"pattern": "FIXME", "path": "src"});
    assert!(!observe(&mut classifier, "grep", &other, &empty).progress);
}

#[test]
fn a_glob_with_no_matches_is_not_progress() {
    let mut classifier = ProgressClassifier::new(limits());
    let arguments = json!({"pattern": "**/*.rs"});
    let empty = json!({"matches": []});
    assert!(!observe(&mut classifier, "glob", &arguments, &empty).progress);
    let found = json!({"matches": ["a.rs"]});
    assert!(observe(&mut classifier, "glob", &arguments, &found).progress);
}

#[test]
fn shell_inspection_commands_follow_the_read_tools() {
    let mut classifier = ProgressClassifier::new(limits());
    let cat_a = json!({"command": "cat a.rs"});
    let cat_b = json!({"command": "cat b.rs"});
    assert!(observe(&mut classifier, "shell", &cat_a, &shell(0, "fn a() {}\n")).progress);
    assert!(!observe(&mut classifier, "shell", &cat_a, &shell(0, "fn a() {}\n")).progress);
    assert!(observe(&mut classifier, "shell", &cat_b, &shell(0, "fn b() {}\n")).progress);
    let ls = json!({"command": "ls -la src"});
    assert!(observe(&mut classifier, "shell", &ls, &shell(0, "total 8\n")).progress);
    assert!(!observe(&mut classifier, "shell", &ls, &shell(0, "total 8\n")).progress);
    // A search that finds nothing moved nothing.
    let grep = json!({"command": "grep -rn TODO src"});
    assert!(!observe(&mut classifier, "shell", &grep, &shell(1, "")).progress);
}

#[test]
fn sed_with_n_is_an_inspection_read() {
    let mut classifier = ProgressClassifier::new(limits());
    let command = json!({"command": "sed -n '1,5p' a.rs"});
    assert!(observe(&mut classifier, "shell", &command, &shell(0, "fn a() {}\n")).progress);
    assert!(!observe(&mut classifier, "shell", &command, &shell(0, "fn a() {}\n")).progress);
}

#[test]
fn consecutive_idle_calls_note_twice_then_stop_at_three_times_the_limit() {
    let mut classifier = ProgressClassifier::new(limits());
    let limit = DEFAULT_NO_PROGRESS_TURNS;
    let stop_at = limit * 3;
    let mut notes = Vec::new();
    // The first `stop_at - 1` calls carry at most the two notes.
    for index in 0..(stop_at - 1) {
        let arguments = json!({"pattern": format!("absent-{index}")});
        let observed = observe(&mut classifier, "grep", &arguments, &json!({"matches": []}));
        assert!(!observed.progress);
        assert!(
            observed.stop.is_none(),
            "call {} is below the stop",
            index + 1
        );
        if let Some(note) = observed.note {
            notes.push((index + 1, note));
        }
    }
    assert_eq!(notes.len(), 2, "one note at the limit and one at twice it");
    assert_eq!(notes[0].0, limit, "{}", notes[0].1);
    assert_eq!(notes[1].0, limit * 2, "{}", notes[1].1);
    assert!(notes[0].1.contains("material progress"), "{}", notes[0].1);
    assert!(notes[1].1.contains("material progress"), "{}", notes[1].1);
    // The call at three times the limit stops the job: the guard counts calls,
    // not turns, so churn hidden inside a productive turn still trips it.
    let arguments = json!({"pattern": "absent-stop"});
    let stop = observe(&mut classifier, "grep", &arguments, &json!({"matches": []}))
        .stop
        .expect("the call at three times the limit stops");
    assert_eq!(stop.kind, ProgressStopKind::NoProgress);
    assert_eq!(stop.measured, u64::from(stop_at));
    assert!(stop.message.contains("calls"), "{}", stop.message);
}

#[test]
fn silent_mutating_commands_do_not_build_the_idle_streak() {
    let mut classifier = ProgressClassifier::new(limits());
    let limit = DEFAULT_NO_PROGRESS_TURNS;
    // Seven idle inspections: one short of the first note.
    for index in 0..(limit - 1) {
        let arguments = json!({"pattern": format!("absent-{index}")});
        let observed = observe(&mut classifier, "grep", &arguments, &json!({"matches": []}));
        assert!(observed.note.is_none(), "call {}", index + 1);
    }
    // A silent, successful, non-read command changed the workspace; it is a
    // neutral step, not an idle one, so the streak stays where it was.
    for command in [
        "chmod +x scripts/check.sh",
        "mkdir -p target/tmp",
        "mv a.rs b.rs",
        "cp a.rs b.rs",
        "touch build.stamp",
        "git add crates/saya-agent/src/lib.rs",
        "git commit -m 'fix: guard'",
        "cargo install --path .",
        "npm install",
    ] {
        let arguments = json!({"command": command});
        let observed = observe(&mut classifier, "shell", &arguments, &shell(0, ""));
        assert!(!observed.progress, "{command}");
        assert!(observed.note.is_none(), "{command}");
        assert!(observed.stop.is_none(), "{command}");
    }
    // Still at seven: the next idle inspection is the call at the limit.
    let arguments = json!({"pattern": "absent-next"});
    assert!(
        observe(&mut classifier, "grep", &arguments, &json!({"matches": []}))
            .note
            .is_some(),
        "the neutral steps did not advance the streak"
    );
}

#[test]
fn repeated_silent_mutating_commands_never_trip_the_guard() {
    let mut classifier = ProgressClassifier::new(limits());
    let arguments = json!({"command": "touch build.stamp"});
    for _ in 0..(DEFAULT_NO_PROGRESS_TURNS * 3 + 2) {
        let observed = observe(&mut classifier, "shell", &arguments, &shell(0, ""));
        assert!(!observed.progress);
        assert!(observed.note.is_none(), "a neutral step is not a repeat");
        assert!(observed.stop.is_none());
    }
}

#[test]
fn a_realistic_legitimate_sequence_never_trips_the_guard() {
    let mut classifier = ProgressClassifier::new(limits());
    let read = json!({"path": "crates/saya-agent/src/lib.rs"});
    let search = json!({"pattern": "TODO", "path": "crates/saya-agent"});
    let found = json!({"matches": [{"path": "a.rs", "line": 1}]});
    let first_edit = json!({"path": "a.rs", "old": "TODO", "new": "done"});
    let chmod = json!({"command": "chmod +x scripts/check.sh"});
    let tests = json!({"command": "cargo test -p saya-agent"});
    let second_edit = json!({"path": "a.rs", "old": "done", "new": "fixed"});
    let steps = [
        observe(
            &mut classifier,
            "read",
            &read,
            &json!({"content": "fn a() {}"}),
        ),
        observe(&mut classifier, "grep", &search, &found),
        observe(&mut classifier, "edit", &first_edit, &json!({"ok": true})),
        observe(&mut classifier, "shell", &chmod, &shell(0, "")),
        observe(
            &mut classifier,
            "shell",
            &tests,
            &shell(1, "test result: FAILED. 3 passed; 1 failed"),
        ),
        observe(&mut classifier, "edit", &second_edit, &json!({"ok": true})),
        observe(
            &mut classifier,
            "shell",
            &tests,
            &shell(0, "test result: ok. 4 passed; 0 failed"),
        ),
    ];
    for (index, observed) in steps.iter().enumerate() {
        assert!(observed.note.is_none(), "step {index}: {:?}", observed.note);
        assert!(observed.stop.is_none(), "step {index}");
    }
}

#[test]
fn a_real_edit_resets_the_churn_streak() {
    let mut classifier = ProgressClassifier::new(limits());
    for index in 0..(DEFAULT_NO_PROGRESS_TURNS - 1) {
        let arguments = json!({"pattern": format!("absent-{index}")});
        assert!(
            observe(&mut classifier, "grep", &arguments, &json!({"matches": []}))
                .note
                .is_none()
        );
    }
    let write = json!({"path": "a.rs", "content": "fixed"});
    assert!(observe(&mut classifier, "write", &write, &json!({"digest": "d"})).progress);
    // The streak restarted: a fresh limit of idle calls notes only at the
    // limit, and never before it.
    for index in 0..(DEFAULT_NO_PROGRESS_TURNS - 1) {
        let arguments = json!({"pattern": format!("again-{index}")});
        let observed = observe(&mut classifier, "grep", &arguments, &json!({"matches": []}));
        assert!(observed.note.is_none(), "call {index}");
        assert!(observed.stop.is_none(), "call {index}");
    }
    let arguments = json!({"pattern": "again-last"});
    assert!(
        observe(&mut classifier, "grep", &arguments, &json!({"matches": []}))
            .note
            .is_some()
    );
}

#[test]
fn semantic_repeat_keys_ignore_incidental_arguments() {
    let mut classifier = ProgressClassifier::new(limits());
    let result = shell(0, "hi\n");
    let first = json!({"command": "echo hi", "description": "one"});
    let second = json!({"command": "echo  hi", "description": "two"});
    assert!(observe(&mut classifier, "shell", &first, &result).progress);
    assert!(
        observe(&mut classifier, "shell", &second, &result)
            .note
            .is_none()
    );
    let noted = observe(&mut classifier, "shell", &second, &result);
    assert_eq!(
        noted.note.as_deref(),
        Some("You are repeating shell with the same result; change approach or finish."),
        "the whitespace and description are not a new call"
    );
}

#[test]
fn a_read_repeat_key_is_the_file_range() {
    let mut classifier = ProgressClassifier::new(limits());
    let result = json!({"content": "same"});
    let range = json!({"path": "a.rs", "start_line": 1, "line_count": 10});
    let shifted = json!({"path": "a.rs", "start_line": 20, "line_count": 10});
    for _ in 0..DEFAULT_REPEAT_LIMIT {
        observe(&mut classifier, "read", &range, &result);
    }
    // A different window of the same file is a different target, not a repeat
    // of this one.
    let other = observe(&mut classifier, "read", &shifted, &result);
    assert!(other.note.is_none() && other.stop.is_none());
}
