//! The change one file-tool call states (UI5): the hunk a `edit` makes, the
//! rows a `write` puts down, and the bounds that keep a hostile change from
//! costing a frame — a size row for a change too large or not text, and
//! `+K more lines` once the rows run past what a pane shows.

use serde_json::Value;

use super::{Diff, MAX_ROWS, of};

/// The change `tool`'s call states, or a panic naming the call that did not.
fn diff(arguments: Value) -> Diff {
    of(&arguments).unwrap_or_else(|| panic!("{arguments} states a change"))
}

/// Every row of a diff that is a body line of `sign`.
fn signed(rows: &[String], sign: char) -> Vec<String> {
    rows.iter()
        .filter(|row| row.starts_with(sign))
        .cloned()
        .collect()
}

#[test]
fn an_anchored_replace_states_the_line_it_takes_and_the_line_it_writes() {
    let change = diff(serde_json::json!({
        "path": "src/calc.py",
        "old_text": "return total - discount - discount",
        "new_text": "return total - discount",
        "expected_digest": "sha256:cafe",
    }));

    assert_eq!(
        change.rows[0], "@@ -1,1 +1,1 @@",
        "a hunk header states both sides, as a patch does"
    );
    assert_eq!(
        signed(&change.rows, '-'),
        ["-return total - discount - discount"],
        "the anchor the call takes away is the minus side: {:?}",
        change.rows
    );
    assert_eq!(
        signed(&change.rows, '+'),
        ["+return total - discount"],
        "the text the call writes is the plus side: {:?}",
        change.rows
    );
    assert_eq!(
        (change.added, change.removed),
        (1, 1),
        "one line written, one taken away"
    );
    assert!(
        !change.rows.join("\n").contains("cafe"),
        "the digest is not part of the change"
    );
}

#[test]
fn a_whole_file_write_states_every_row_it_puts_down() {
    let change = diff(serde_json::json!({"path": "notes.md", "content": "one\ntwo\n"}));

    assert_eq!(
        change.rows,
        ["@@ -0,0 +1,2 @@", "+one", "+two"],
        "a file written from nothing is every row added"
    );
    assert_eq!((change.added, change.removed), (2, 0));
}

#[test]
fn a_multi_edit_states_every_pair_it_carries() {
    let change = diff(serde_json::json!({
        "path": "a.py",
        "edits": [
            {"old_text": "one", "new_text": "ONE"},
            {"old_text": "two", "new_text": "TWO"},
        ],
    }));

    assert_eq!(
        signed(&change.rows, '-'),
        ["-one", "-two"],
        "both anchors are on the minus side: {:?}",
        change.rows
    );
    assert_eq!(signed(&change.rows, '+'), ["+ONE", "+TWO"]);
    assert_eq!((change.added, change.removed), (2, 2));
}

#[test]
fn an_append_states_only_what_it_writes() {
    let change = diff(serde_json::json!({"path": "a.py", "offset": 42, "chunk": "three\n"}));

    assert_eq!(
        change.rows,
        ["@@ -0,0 +1,1 @@", "+three"],
        "an append takes nothing away: {:?}",
        change.rows
    );
    assert_eq!((change.added, change.removed), (1, 0));
}

#[test]
fn a_change_that_is_not_text_is_one_size_row() {
    let change = diff(serde_json::json!({"path": "logo.png", "content": "abc\u{0}def"}));

    assert_eq!(
        change.rows.len(),
        1,
        "a binary change is stated, never painted: {:?}",
        change.rows
    );
    assert!(
        change.rows[0].contains("binary") && change.rows[0].contains("7 bytes"),
        "the row states the kind and the size: {:?}",
        change.rows
    );
    assert!(
        change
            .rows
            .iter()
            .all(|row| !row.starts_with('+') && !row.starts_with('-')),
        "no line of it reaches the pane: {:?}",
        change.rows
    );
}

#[test]
fn a_change_over_the_byte_bound_is_one_size_row() {
    let content = "x".repeat(200_000);
    let change = diff(serde_json::json!({"path": "big.txt", "content": content}));

    assert_eq!(
        change.rows.len(),
        1,
        "a huge change is one row, never a table: {}",
        change.rows.len()
    );
    assert!(
        change.rows[0].contains("too large") && change.rows[0].contains("200000 bytes"),
        "the row states the size: {:?}",
        change.rows
    );
    assert!(
        change.added > 0,
        "the counts still state the change: {change:?}"
    );
}

#[test]
fn the_rows_are_bounded_and_say_how_many_are_left() {
    let content = (0..500)
        .map(|line| format!("line {line:03}\n"))
        .collect::<String>();
    let change = diff(serde_json::json!({"path": "long.txt", "content": content}));

    assert_eq!(
        change.rows.len(),
        MAX_ROWS + 1,
        "the rows stop at the bound, one row past it to say so"
    );
    let last = change.rows.last().expect("the last row is the count");
    assert!(
        last.starts_with('+') && last.ends_with("more lines"),
        "the cut is stated as the rows it left out: {last:?}"
    );
    assert!(
        !last.starts_with("+ "),
        "the count is not one of the change's own lines: {last:?}"
    );
    assert_eq!(
        change.added, 500,
        "the count states the whole change, not what fits: {change:?}"
    );
}

#[test]
fn a_control_byte_in_the_change_never_reaches_the_pane() {
    let change = diff(serde_json::json!({
        "path": "a.py",
        "old_text": "one\u{1b}[2J",
        "new_text": "two\u{7}",
    }));
    let rows = change.rows.join("\n");

    assert!(
        !rows.contains('\u{1b}') && !rows.contains('\u{7}'),
        "a control byte is replaced before the diff is built: {rows:?}"
    );
    assert!(
        rows.contains("one") && rows.contains("two"),
        "the words themselves stay: {rows}"
    );
}

#[test]
fn a_call_that_states_no_change_states_nothing() {
    for arguments in [
        serde_json::json!({"path": "a.py"}),
        serde_json::json!({"command": "ls"}),
        serde_json::json!({"content": "one"}),
        serde_json::json!({"path": "a.py", "old_text": 7, "new_text": 8}),
        serde_json::json!({"path": "a.py", "edits": []}),
        serde_json::json!({"path": "a.py", "chunk": "three"}),
        serde_json::json!({"path": "a.py", "source": "a notebook cell"}),
    ] {
        assert_eq!(
            of(&arguments),
            None,
            "a call with no change to state states none: {arguments}"
        );
    }
}
