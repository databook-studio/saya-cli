//! Tests for the bounded diff: exact unified text, the created/missing
//! labels, every size/line/cell/output bound, and pathological shapes
//! that must stay fast and bounded.

use std::fs;
use std::time::Instant;

use super::diff::{DIFF_MAX_BYTES, DIFF_MAX_LINES, DIFF_MAX_OUTPUT_LINES};
use super::testing::*;
use super::*;
use saya_types::EditSource;

#[test]
fn identical_sides_report_nothing_to_diff() {
    assert_eq!(
        render_diff(b"a\nb\n", Some(b"a\nb\n"), "f.txt"),
        DiffOutcome::NothingToDiff
    );
    assert_eq!(
        render_diff(b"", Some(b""), "f.txt"),
        DiffOutcome::NothingToDiff
    );
}

#[test]
fn one_changed_line_pins_the_exact_unified_text() {
    let before: &[u8] = b"line one\nold two\nline three\n";
    let current: &[u8] = b"line one\nnew two\nline three\n";
    let outcome = render_diff(before, Some(current), "notes.txt");
    assert_eq!(
        outcome,
        DiffOutcome::Diff {
            text: concat!(
                "diff preview: notes.txt\n",
                "--- before (recorded)\n",
                "+++ current\n",
                "@@ -1,3 +1,3 @@\n",
                " line one\n",
                "-old two\n",
                "+new two\n",
                " line three",
            )
            .to_owned(),
            truncated: false,
        }
    );
}

#[test]
fn added_and_removed_lines_render_one_hunk() {
    let before: &[u8] = b"a\nb\nc\n";
    let current: &[u8] = b"a\nx\ny\nc\nz\n";
    let outcome = render_diff(before, Some(current), "seq.txt");
    assert_eq!(
        outcome,
        DiffOutcome::Diff {
            text: concat!(
                "diff preview: seq.txt\n",
                "--- before (recorded)\n",
                "+++ current\n",
                "@@ -1,3 +1,5 @@\n",
                " a\n",
                "-b\n",
                "+x\n",
                "+y\n",
                " c\n",
                "+z",
            )
            .to_owned(),
            truncated: false,
        }
    );
}

#[test]
fn separated_changes_render_two_hunks() {
    let before: &[u8] = b"a\nb\nc\nd\ne\nf\ng\n";
    let current: &[u8] = b"A\nb\nc\nd\ne\nf\nG\n";
    let outcome = render_diff(before, Some(current), "seq.txt");
    assert_eq!(
        outcome,
        DiffOutcome::Diff {
            text: concat!(
                "diff preview: seq.txt\n",
                "--- before (recorded)\n",
                "+++ current\n",
                "@@ -1,3 +1,3 @@\n",
                "-a\n",
                "+A\n",
                " b\n",
                " c\n",
                "@@ -5,3 +5,3 @@\n",
                " e\n",
                " f\n",
                "-g\n",
                "+G",
            )
            .to_owned(),
            truncated: false,
        }
    );
}

#[test]
fn crlf_and_missing_trailing_newline_render_deterministically() {
    let crlf = render_diff(b"x\r\ny\r\n", Some(b"x\r\nz\r\n"), "crlf.txt");
    assert_eq!(
        crlf,
        DiffOutcome::Diff {
            text: "diff preview: crlf.txt\n--- before (recorded)\n+++ current\n@@ -1,2 +1,2 @@\n x\r\n-y\r\n+z\r"
                .to_owned(),
            truncated: false,
        }
    );
    let without = render_diff(b"x\ny", Some(b"x\nz"), "eof.txt");
    assert_eq!(
        without,
        DiffOutcome::Diff {
            text: concat!(
                "diff preview: eof.txt\n",
                "--- before (recorded)\n",
                "+++ current\n",
                "@@ -1,2 +1,2 @@\n",
                " x\n",
                "-y\n",
                "\\ No newline at end of file\n",
                "+z\n",
                "\\ No newline at end of file",
            )
            .to_owned(),
            truncated: false,
        }
    );
    let with = render_diff(b"x\ny\n", Some(b"x\nz\n"), "eof.txt");
    assert_eq!(
        with,
        DiffOutcome::Diff {
            text: concat!(
                "diff preview: eof.txt\n",
                "--- before (recorded)\n",
                "+++ current\n",
                "@@ -1,2 +1,2 @@\n",
                " x\n",
                "-y\n",
                "+z",
            )
            .to_owned(),
            truncated: false,
        }
    );
    assert_eq!(
        render_diff(b"a\n", Some(b"a"), "eof.txt"),
        DiffOutcome::Diff {
            text: concat!(
                "diff preview: eof.txt\n",
                "--- before (recorded)\n",
                "+++ current\n",
                "@@ -1,1 +1,1 @@\n",
                "-a\n",
                "+a\n",
                "\\ No newline at end of file",
            )
            .to_owned(),
            truncated: false,
        }
    );
}

#[test]
fn binary_or_nul_bytes_refuse_the_diff_with_sizes() {
    assert_eq!(
        render_diff(b"\xff\xfe\xfd", Some(b"a\n"), "bin.dat"),
        DiffOutcome::BinaryOrNonText {
            before_bytes: 3,
            current_bytes: 2,
        }
    );
    assert_eq!(
        render_diff(b"a\0b", Some(b"x\n"), "nul.txt"),
        DiffOutcome::BinaryOrNonText {
            before_bytes: 3,
            current_bytes: 2,
        }
    );
}

#[test]
fn the_byte_bound_refuses_with_sizes() {
    let big = vec![b'x'; DIFF_MAX_BYTES + 1];
    assert_eq!(
        render_diff(&big, Some(b"y\n"), "big.bin"),
        DiffOutcome::TooLargeToDiff {
            before_bytes: DIFF_MAX_BYTES + 1,
            current_bytes: 2,
        }
    );
    let at_bound = vec![b'x'; DIFF_MAX_BYTES];
    assert!(matches!(
        render_diff(&at_bound, Some(b"y\n"), "edge.bin"),
        DiffOutcome::Diff { .. }
    ));
}

#[test]
fn the_line_bound_refuses_with_sizes() {
    let many = "line\n".repeat(DIFF_MAX_LINES + 1).into_bytes();
    assert_eq!(
        render_diff(&many, Some(b"one\n"), "many.txt"),
        DiffOutcome::TooLargeToDiff {
            before_bytes: many.len(),
            current_bytes: 4,
        }
    );
}

#[test]
fn the_cell_bound_refuses_two_thousand_identical_lines() {
    let table = "same line\n".repeat(DIFF_MAX_LINES).into_bytes();
    assert_eq!(
        render_diff(&table, Some(&table), "same.txt"),
        DiffOutcome::TooLargeToDiff {
            before_bytes: table.len(),
            current_bytes: table.len(),
        }
    );
}

#[test]
fn the_output_cap_truncates_two_thousand_changed_lines() {
    let before = (0..DIFF_MAX_LINES)
        .map(|i| format!("l{i}\n"))
        .collect::<String>()
        .into_bytes();
    let current = (0..DIFF_MAX_LINES - 2)
        .map(|i| format!("m{i}\n"))
        .collect::<String>()
        .into_bytes();
    let outcome = render_diff(&before, Some(&current), "many.txt");
    let DiffOutcome::Diff { text, truncated } = outcome else {
        panic!("expected a diff, got {outcome:?}");
    };
    assert!(truncated);
    assert_eq!(text.lines().count(), 3 + DIFF_MAX_OUTPUT_LINES + 1);
    assert_eq!(
        text.lines().last(),
        Some("… diff truncated (3799 more lines)")
    );
}

#[test]
fn pathological_shapes_stay_fast_and_bounded() {
    let identical = "same line\n".repeat(DIFF_MAX_LINES).into_bytes();
    let started = Instant::now();
    let outcome = render_diff(&identical, Some(&identical), "same.txt");
    assert!(matches!(outcome, DiffOutcome::TooLargeToDiff { .. }));
    assert!(started.elapsed().as_secs() < 2);

    let before = (0..DIFF_MAX_LINES)
        .map(|i| format!("l{i}\n"))
        .collect::<String>()
        .into_bytes();
    let current = (0..DIFF_MAX_LINES - 2)
        .map(|i| format!("m{i}\n"))
        .collect::<String>()
        .into_bytes();
    let started = Instant::now();
    let outcome = render_diff(&before, Some(&current), "many.txt");
    assert!(matches!(
        outcome,
        DiffOutcome::Diff {
            truncated: true,
            ..
        }
    ));
    assert!(started.elapsed().as_secs() < 2);

    let one_line = vec![b'a'; DIFF_MAX_BYTES];
    let other_line = vec![b'b'; DIFF_MAX_BYTES];
    let started = Instant::now();
    let outcome = render_diff(&one_line, Some(&other_line), "wide.txt");
    assert!(matches!(
        outcome,
        DiffOutcome::Diff {
            truncated: false,
            ..
        }
    ));
    assert!(started.elapsed().as_secs() < 2);
}

#[test]
fn diff_change_labels_a_created_file_and_a_missing_file() {
    let sandbox = Sandbox::new("diff-labels");
    let created: &[u8] = b"alpha\nbeta\n";
    let record = record_commit(
        &sandbox,
        "made.txt",
        EditSource::WorkspaceWrite,
        None,
        created,
        1_000,
    );
    fs::write(sandbox.ws_root().join("made.txt"), created).expect("file");
    let outcome = diff_change(
        &sandbox.store,
        SESSION,
        Some(&sandbox.ws),
        record.id.as_str(),
    );
    let DiffOutcome::Diff { text, truncated } = &outcome else {
        panic!("expected a diff, got {outcome:?}");
    };
    assert!(!*truncated);
    assert_eq!(
        text.lines().take(4).last(),
        Some("before is empty (created by the edit)")
    );
    assert!(text.contains("(created by the edit)"), "{text}");
    assert!(text.contains("+alpha") && text.contains("+beta"), "{text}");

    let v1: &[u8] = b"before\n";
    let v2: &[u8] = b"after\n";
    fs::write(sandbox.ws_root().join("notes.txt"), v1).expect("seed");
    let replace = record_commit(
        &sandbox,
        "notes.txt",
        EditSource::WorkspaceEdit,
        Some(v1),
        v2,
        2_000,
    );
    fs::write(sandbox.ws_root().join("notes.txt"), v2).expect("edit");
    fs::remove_file(sandbox.ws_root().join("notes.txt")).expect("delete");
    let outcome = diff_change(
        &sandbox.store,
        SESSION,
        Some(&sandbox.ws),
        replace.id.as_str(),
    );
    let DiffOutcome::Diff { text, .. } = &outcome else {
        panic!("expected a diff, got {outcome:?}");
    };
    assert!(text.contains("(file is missing now)"), "{text}");
    assert!(
        text.lines().any(|line| line == "-before"),
        "the missing file's recorded content shows as removed: {text}"
    );
}

#[test]
fn diff_change_of_a_reverted_file_reports_nothing_to_diff() {
    let sandbox = Sandbox::new("diff-same");
    let v1: &[u8] = b"before\n";
    let v2: &[u8] = b"after\n";
    fs::write(sandbox.ws_root().join("notes.txt"), v1).expect("seed");
    let record = record_commit(
        &sandbox,
        "notes.txt",
        EditSource::WorkspaceEdit,
        Some(v1),
        v2,
        1_000,
    );
    fs::write(sandbox.ws_root().join("notes.txt"), v2).expect("edit");
    fs::write(sandbox.ws_root().join("notes.txt"), v1).expect("revert");
    assert_eq!(
        diff_change(
            &sandbox.store,
            SESSION,
            Some(&sandbox.ws),
            record.id.as_str()
        ),
        DiffOutcome::NothingToDiff
    );
}

#[test]
fn diff_change_refuses_without_a_workspace_or_an_id() {
    let sandbox = Sandbox::new("diff-refuse");
    let record = record_commit(
        &sandbox,
        "notes.txt",
        EditSource::WorkspaceWrite,
        None,
        b"x",
        1_000,
    );
    assert_eq!(
        diff_change(&sandbox.store, SESSION, None, record.id.as_str()),
        DiffOutcome::Refused(RestoreRefusal::NoWorkspace)
    );
    assert_eq!(
        diff_change(&sandbox.store, SESSION, Some(&sandbox.ws), "junk"),
        DiffOutcome::Refused(RestoreRefusal::NotFound)
    );
    assert_eq!(
        diff_change(
            &sandbox.store,
            SESSION,
            Some(&sandbox.ws),
            &format!("ck-{}", "cd".repeat(16)),
        ),
        DiffOutcome::Refused(RestoreRefusal::NotFound)
    );
}

#[test]
fn diff_change_refuses_a_tampered_before_image() {
    let sandbox = Sandbox::new("diff-tamper");
    let v1: &[u8] = b"before\n";
    let v2: &[u8] = b"after\n";
    fs::write(sandbox.ws_root().join("notes.txt"), v1).expect("seed");
    let record = record_commit(
        &sandbox,
        "notes.txt",
        EditSource::WorkspaceEdit,
        Some(v1),
        v2,
        1_000,
    );
    fs::write(sandbox.ws_root().join("notes.txt"), v2).expect("edit");
    let image = sandbox
        .checkpoints_dir()
        .join(format!("{}.before", record.id.as_str()));
    fs::write(&image, b"tampered bytes").expect("tamper");
    assert_eq!(
        diff_change(
            &sandbox.store,
            SESSION,
            Some(&sandbox.ws),
            record.id.as_str()
        ),
        DiffOutcome::Refused(RestoreRefusal::BeforeImageCorrupt)
    );
}
