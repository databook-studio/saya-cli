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
