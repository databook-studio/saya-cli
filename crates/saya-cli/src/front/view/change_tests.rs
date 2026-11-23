//! A turn's changed files (UI5): the two edits the `Changed:` line names, the
//! calls that are not changes, the file a turn writes twice, and the count the
//! line folds when a turn touches more files than it can name.

use serde_json::Value;

use super::change::{Change, MAX_FILES, changed, last_changed, summary};
use super::entry::{Activity, Entry, Step, StepState, TurnView};

/// A step of a turn: its tool, the call's own bytes, and how it ended.
fn step(tool: &str, arguments: Value, state: StepState) -> Step {
    Step {
        tool: tool.to_owned(),
        arguments,
        state,
    }
}

/// A landed file call.
fn done(tool: &str, arguments: Value) -> Step {
    step(
        tool,
        arguments,
        StepState::Done {
            summary: String::from("workspace file edited"),
        },
    )
}

/// A turn whose activity block holds `steps`.
fn turn_of(steps: Vec<Step>) -> TurnView {
    let mut turn = TurnView::default();
    turn.push(Entry::Activity(Activity {
        steps,
        ended: true,
        ..Activity::default()
    }));
    turn
}

/// Two files a turn edited: a replace with one line each way, and a write.
fn two_files() -> TurnView {
    turn_of(vec![
        done(
            "edit",
            serde_json::json!({
                "path": "src/calc.py",
                "old_text": "return total - discount - discount",
                "new_text": "return total - discount",
            }),
        ),
        done(
            "write",
            serde_json::json!({"path": "notes/todo.md", "content": "one\ntwo\nthree\n"}),
        ),
    ])
}

#[test]
fn a_turn_that_changed_two_files_states_both_with_their_counts() {
    let files = changed(&two_files());

    assert_eq!(
        files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        ["src/calc.py", "notes/todo.md"],
        "each file once, in the order the turn first named it"
    );
    assert_eq!(
        (files[0].added, files[0].removed),
        (1, 1),
        "the replace writes one line and takes one away"
    );
    assert_eq!(
        (files[1].added, files[1].removed),
        (3, 0),
        "the write puts three lines down"
    );
    assert!(
        files[0].rows.iter().any(|row| row.starts_with('+')),
        "the pane has the replace's rows to paint: {:?}",
        files[0].rows
    );
}

#[test]
fn the_changed_line_names_the_files_the_counts_and_the_key() {
    let line = summary(&changed(&two_files())).expect("two changed files are stated");

    assert_eq!(
        line, "Changed: src/calc.py (+1 −1), notes/todo.md (+3 −0) · Ctrl+D diff",
        "one line, every file, both counts, and the key that opens the rest"
    );
}

#[test]
fn a_turn_that_changed_no_file_says_nothing() {
    let turn = turn_of(vec![
        done("read", serde_json::json!({"path": "src/calc.py"})),
        done("grep", serde_json::json!({"pattern": "discount"})),
        done("shell", serde_json::json!({"command": "ls"})),
    ]);

    assert!(changed(&turn).is_empty(), "no call changed a file");
    assert_eq!(summary(&changed(&turn)), None, "and nothing is claimed");
}

#[test]
fn a_call_that_did_not_land_is_not_a_change() {
    for state in [
        StepState::Failed {
            summary: String::from("refused"),
        },
        StepState::Denied {
            reason: String::from("denied by the operator"),
        },
        StepState::Running,
    ] {
        let turn = turn_of(vec![step(
            "write",
            serde_json::json!({"path": "a.py", "content": "one"}),
            state.clone(),
        )]);
        assert!(
            changed(&turn).is_empty(),
            "a call that is not landed changed nothing: {state:?}"
        );
    }
}

#[test]
fn two_calls_to_one_file_are_one_entry() {
    let turn = turn_of(vec![
        done(
            "edit",
            serde_json::json!({"path": "a.py", "old_text": "one", "new_text": "ONE"}),
        ),
        done(
            "edit",
            serde_json::json!({"path": "a.py", "old_text": "two", "new_text": "TWO"}),
        ),
    ]);
    let files = changed(&turn);

    assert_eq!(files.len(), 1, "the operator reads a file, not a call");
    assert_eq!((files[0].added, files[0].removed), (2, 2));
    assert_eq!(
        summary(&files).as_deref(),
        Some("Changed: a.py (+2 −2) · Ctrl+D diff"),
        "the counts are the turn's whole change to that file"
    );
}

#[test]
fn a_long_line_names_what_fits_and_counts_the_rest() {
    let steps = (0..MAX_FILES + 2)
        .map(|index| {
            done(
                "write",
                serde_json::json!({"path": format!("f{index}.md"), "content": "one\n"}),
            )
        })
        .collect();
    let line = summary(&changed(&turn_of(steps))).expect("a turn of writes is stated");

    for index in 0..MAX_FILES {
        assert!(
            line.contains(&format!("f{index}.md")),
            "the first files are named: {line}"
        );
    }
    assert!(
        line.contains("+2 more files"),
        "and the rest are counted, never dropped: {line}"
    );
}

#[test]
fn the_pane_opens_on_the_newest_turn_that_changed_a_file() {
    let quiet = turn_of(vec![done(
        "read",
        serde_json::json!({"path": "src/calc.py"}),
    )]);
    let turns = vec![two_files(), quiet];

    assert_eq!(
        last_changed(&turns)
            .iter()
            .map(|file| file.path.clone())
            .collect::<Vec<_>>(),
        ["src/calc.py", "notes/todo.md"],
        "the last turn that wrote is the one the pane opens"
    );
    assert!(
        last_changed(&[turn_of(Vec::new())]).is_empty(),
        "a session that never wrote has nothing to open"
    );
}

#[test]
fn a_change_that_is_one_size_row_is_still_one_file() {
    let turn = turn_of(vec![done(
        "write",
        serde_json::json!({"path": "logo.png", "content": "abc\u{0}def"}),
    )]);
    let files: Vec<Change> = changed(&turn);

    assert_eq!(files.len(), 1);
    assert!(
        files[0].rows[0].contains("binary"),
        "the pane states the size row instead of bytes: {:?}",
        files[0].rows
    );
}
