/// A tight budget drops trailing file/check lines first and ends with the
/// visible `more not shown` note; the body never exceeds the cap, and a
/// budget too small even for the header truncates with the flag set.
#[test]
fn a_tight_budget_drops_trailing_lines_and_notes_the_cut() {
    let (root, workspace) = workspace("budget");
    let files = (0..6)
        .map(|index| file_fact(&format!("file{index}.txt"), None, TWO_MINUTES_AGO))
        .collect::<Vec<_>>();
    let checks = (0..4)
        .map(|index| {
            check_fact(
                &format!("check number {index}"),
                CheckOutcome::Passed,
                TWO_MINUTES_AGO,
            )
        })
        .collect();
    let record = current_record(Some("a short note"), files, checks);
    let block = render_record_block_at(Some(&record), Some(&workspace), NOW, 600)
        .expect("content with budget carries a block");
    assert!(
        block.body.len() <= 600,
        "the body must fit the budget: {} bytes",
        block.body.len()
    );
    assert!(block.body.starts_with(HEADER), "body: {}", block.body);
    assert!(block.body.contains("a short note"), "body: {}", block.body);
    assert!(
        block.body.contains("more not shown"),
        "a cut must be visible: {}",
        block.body
    );
    assert!(
        block.body.contains("- file0.txt"),
        "dropping is trailing-first: {}",
        block.body
    );
    assert!(
        !block.body.contains("- file5.txt"),
        "the last file line drops first: {}",
        block.body
    );
    assert!(
        !block.body.contains("check number 3"),
        "checks drop before files: {}",
        block.body
    );
    assert!(!block.truncated, "the fitted body needs no byte cut");

    // A budget smaller than the header alone: everything is cut, the flag
    // fires, and the body still never exceeds the cap.
    let cut = render_record_block_at(Some(&record), Some(&workspace), NOW, 64)
        .expect("a tiny budget still yields the block");
    assert!(cut.truncated, "a byte-cut body is flagged: {}", cut.body);
    assert!(cut.body.len() <= 64, "body: {} bytes", cut.body.len());
    assert!(
        cut.body.len() <= MAX_RECORD_BLOCK_BYTES,
        "the block cap holds: {} bytes",
        cut.body.len()
    );
    let _ = std::fs::remove_dir_all(root);
}

/// Free text is redacted in the body — secret-shaped check labels and notes
/// never reach the model verbatim, even in a hand-built record.
#[test]
fn record_text_is_redacted_in_the_body() {
    let (root, workspace) = workspace("redact");
    let record = current_record(
        Some("run with token=abc123"),
        Vec::new(),
        vec![check_fact(
            "password=hunter2",
            CheckOutcome::Passed,
            TWO_MINUTES_AGO,
        )],
    );
    let block = render_record_block_at(Some(&record), Some(&workspace), NOW, 16 * 1024)
        .expect("content carries a block");
    assert!(
        !block.body.contains("hunter2"),
        "a secret-shaped label leaked: {}",
        block.body
    );
    assert!(
        block.body.contains("password=[redacted]"),
        "body: {}",
        block.body
    );
    assert!(
        !block.body.contains("abc123"),
        "a secret-shaped note leaked: {}",
        block.body
    );
    assert!(
        block.body.contains("token=[redacted]"),
        "body: {}",
        block.body
    );
    let _ = std::fs::remove_dir_all(root);
}
