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
