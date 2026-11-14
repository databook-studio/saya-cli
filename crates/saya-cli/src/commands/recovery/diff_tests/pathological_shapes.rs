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
