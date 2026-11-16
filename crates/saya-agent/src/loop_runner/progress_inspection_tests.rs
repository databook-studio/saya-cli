//! Ranged reads: a shell inspection is keyed by its file and its range or
//! arguments, a changed result for the same key is progress, and a `cd`
//! prefix does not change what was read. Paging a large file, re-tailing a
//! growing log, and zero-match searches interleaved with progress never trip
//! the turn-level guard.

use serde_json::json;

use super::ProgressClassifier;
use super::inspection::parse;
use super::tests::{limits, observe};

fn shell(exit: i64, stdout: &str) -> serde_json::Value {
    json!({"exit_code": exit, "stdout": stdout, "stderr": ""})
}

#[test]
fn inspection_keys_carry_the_range_and_the_file() {
    let head_20 = parse("head -n 20 build.log").expect("head is an inspection");
    let head_40 = parse("head -n 40 build.log").expect("head is an inspection");
    assert_ne!(
        head_20.key(),
        head_40.key(),
        "the range is part of the target"
    );

    let sed_first = parse("sed -n '1,200p' big.txt").expect("sed -n is an inspection");
    let sed_next = parse("sed -n '201,400p' big.txt").expect("sed -n is an inspection");
    assert_ne!(
        sed_first.key(),
        sed_next.key(),
        "a different range of the same file is a different target"
    );
    assert_ne!(
        sed_first.key(),
        parse("sed -n '1,200p' other.txt")
            .expect("sed -n is an inspection")
            .key(),
        "a different file is a different target"
    );
}

#[test]
fn tail_follow_is_not_an_inspection() {
    for command in [
        "tail -f build.log",
        "tail -F build.log",
        "tail --follow build.log",
        "tail -n 20 -f build.log",
    ] {
        assert!(
            parse(command).is_none(),
            "{command} follows the file and must not be keyed as a read"
        );
    }
    assert!(parse("tail -n 20 build.log").is_some());
}

#[test]
fn a_cd_prefix_parses_as_the_same_inspection() {
    let bare = parse("sed -n '1,5p' a.rs").expect("sed -n is an inspection");
    let prefixed = parse("cd crates/agent && sed -n '1,5p' a.rs")
        .expect("the same inspection after a cd is still an inspection");
    assert_eq!(
        bare.key(),
        prefixed.key(),
        "cd changes the directory, not what was read"
    );
}

#[test]
fn paging_a_large_file_in_chunks_is_progress_every_turn() {
    let mut classifier = ProgressClassifier::new(limits());
    let mut turns = 0;
    for start in (1..=3_000).step_by(200) {
        let end = start + 199;
        let arguments = json!({"command": format!("sed -n '{start},{end}p' big.txt")});
        let observed = observe(&mut classifier, "shell", &arguments, &shell(0, "line\n"));
        assert!(observed.progress, "the chunk at {start} is a new read");
        assert_eq!(
            classifier.finish_turn(None),
            None,
            "the chunk at {start} must not count as a stuck turn"
        );
        turns += 1;
    }
    assert_eq!(turns, 15);
    assert_eq!(classifier.report().last_progress_turn, Some(15));
}

#[test]
fn re_tailing_a_growing_log_is_progress_on_every_changed_read() {
    let mut classifier = ProgressClassifier::new(limits());
    let arguments = json!({"command": "tail -20 build.log"});
    for turn in 1..=12 {
        let output = format!("line {turn}\n");
        let observed = observe(&mut classifier, "shell", &arguments, &shell(0, &output));
        assert!(
            observed.progress,
            "the log grew, so the read is progress (turn {turn})"
        );
        assert_eq!(classifier.finish_turn(None), None, "turn {turn}");
    }
    // The same tail read twice with the same bytes is not progress.
    let steady = observe(&mut classifier, "shell", &arguments, &shell(0, "line 12\n"));
    assert!(!steady.progress);
}

#[test]
fn a_changed_inspection_result_for_the_same_range_is_progress() {
    let mut classifier = ProgressClassifier::new(limits());
    let arguments = json!({"command": "head -n 5 a.rs"});
    assert!(observe(&mut classifier, "shell", &arguments, &shell(0, "one\n")).progress);
    assert!(!observe(&mut classifier, "shell", &arguments, &shell(0, "one\n")).progress);
    assert!(
        observe(
            &mut classifier,
            "shell",
            &arguments,
            &shell(0, "one\ntwo\n")
        )
        .progress,
        "the same range returning different bytes moved something"
    );
}

#[test]
fn read_ranges_are_distinct_targets_and_a_changed_result_is_progress() {
    let mut classifier = ProgressClassifier::new(limits());
    let first = json!({"path": "big.rs", "start_line": 1, "line_count": 200});
    let next = json!({"path": "big.rs", "start_line": 201, "line_count": 200});
    let same = json!({"content": "same"});
    assert!(observe(&mut classifier, "read", &first, &same).progress);
    assert!(!observe(&mut classifier, "read", &first, &same).progress);
    assert!(
        observe(&mut classifier, "read", &next, &same).progress,
        "a different range of the same file is a new read"
    );
    assert!(
        observe(
            &mut classifier,
            "read",
            &next,
            &json!({"content": "changed"})
        )
        .progress,
        "the same range returning changed content moved something"
    );
}

#[test]
fn zero_match_searches_interleaved_with_progress_do_not_stop() {
    let mut classifier = ProgressClassifier::new(limits());
    for turn in 0..8 {
        let search = json!({"pattern": format!("absent-{turn}"), "path": "src"});
        let observed = observe(&mut classifier, "grep", &search, &json!({"matches": []}));
        assert!(!observed.progress, "a zero-match search moved nothing");
        // The same turn also writes: the progress clears the streak.
        let write = json!({"path": format!("src/f{turn}.rs"), "content": "x"});
        assert!(observe(&mut classifier, "write", &write, &json!({"ok": true})).progress);
        assert_eq!(classifier.finish_turn(None), None, "turn {turn}");
    }
}
