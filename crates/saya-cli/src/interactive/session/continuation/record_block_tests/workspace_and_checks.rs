/// With no workspace bound there is nothing to refresh: every file reports
/// `not re-checked` and the no-workspace note appears once under the heading.
#[test]
fn without_a_workspace_files_report_not_rechecked() {
    let record = current_record(
        None,
        vec![
            file_fact("a.md", Some("a".repeat(64)), TWO_MINUTES_AGO),
            file_fact("b.md", Some("b".repeat(64)), TWO_MINUTES_AGO),
        ],
        Vec::new(),
    );
    let block = render_record_block_at(Some(&record), None, NOW, 16 * 1024)
        .expect("file facts carry a block even unbound");
    assert!(block.body.contains(FILES_HEADING), "body: {}", block.body);
    assert_eq!(
        block.body.matches(NO_WORKSPACE_LINE).count(),
        1,
        "the no-workspace note appears once: {}",
        block.body
    );
    for line in [
        "- a.md — not re-checked (observed 2m ago)",
        "- b.md — not re-checked (observed 2m ago)",
    ] {
        assert!(
            block.body.contains(line),
            "missing {line:?} in {}",
            block.body
        );
    }
}

/// Checks render under the historical heading with their serde outcome names:
/// they were not re-run and may no longer hold.
#[test]
fn checks_render_under_the_historical_heading() {
    let (root, workspace) = workspace("checks");
    let checks = vec![
        check_fact(
            "cargo test -p saya-types",
            CheckOutcome::Passed,
            TWO_MINUTES_AGO,
        ),
        check_fact(
            "cargo clippy workspace",
            CheckOutcome::Failed,
            TWO_MINUTES_AGO,
        ),
        check_fact("the slow suite", CheckOutcome::TimedOut, TWO_MINUTES_AGO),
    ];
    let record = current_record(None, Vec::new(), checks);
    let block = render_record_block_at(Some(&record), Some(&workspace), NOW, 16 * 1024)
        .expect("check facts carry a block");
    assert!(block.body.contains(CHECKS_HEADING), "body: {}", block.body);
    for line in [
        "- cargo test -p saya-types — passed (observed 2m ago)",
        "- cargo clippy workspace — failed (observed 2m ago)",
        "- the slow suite — timed_out (observed 2m ago)",
    ] {
        assert!(
            block.body.contains(line),
            "missing {line:?} in {}",
            block.body
        );
    }
    let _ = std::fs::remove_dir_all(root);
}

/// The coarse age table: sub-minute, minutes, hours, days — and a future
/// timestamp prints `just now`, never a negative or `ago` form.
#[test]
fn ages_render_as_a_coarse_human_table() {
    let now = 1_700_000_010_000i64;
    assert_eq!(age_text(now - 10_000, now), "<1m");
    assert_eq!(age_text(now, now), "<1m");
    assert_eq!(age_text(now - 59_999, now), "<1m");
    assert_eq!(age_text(now - 60_000, now), "1m");
    assert_eq!(age_text(now - 119_999, now), "1m");
    assert_eq!(age_text(now - 3_599_999, now), "59m");
    assert_eq!(age_text(now - 3_600_000, now), "1h");
    assert_eq!(age_text(now - 86_399_999, now), "23h");
    assert_eq!(age_text(now - 86_400_000, now), "1d");
    assert_eq!(age_text(now - 5 * 86_400_000, now), "5d");
    assert_eq!(age_text(now + 1, now), "just now");

    // The rendered line for a future timestamp: `just now`, with no `ago`.
    let (root, workspace) = workspace("future");
    let record = current_record(
        None,
        vec![file_fact("soon.md", Some("a".repeat(64)), now + 1)],
        Vec::new(),
    );
    let block = render_record_block_at(Some(&record), Some(&workspace), now, 16 * 1024)
        .expect("one file fact carries a block");
    assert!(
        block
            .body
            .contains("- soon.md — not found (observed just now)"),
        "body: {}",
        block.body
    );
    assert!(
        !block.body.contains("just now ago"),
        "a future timestamp never renders `ago`: {}",
        block.body
    );
    let _ = std::fs::remove_dir_all(root);
}
