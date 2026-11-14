/// None, an unsupported (newer binary's) record, a narrative-only record, and
/// a zero budget all render nothing: the block exists only when the record
/// carries notes, files, or checks and there is budget left to show them.
#[test]
fn the_block_is_absent_without_a_usable_record() {
    let (root, workspace) = workspace("absent");
    let budget = 16 * 1024;
    assert!(
        render_record_block_at(None, Some(&workspace), NOW, budget).is_none(),
        "no record, no block"
    );
    let unsupported = StoredContinuation::Unsupported(serde_json::json!({"version": 99}));
    assert!(
        render_record_block_at(Some(&unsupported), Some(&workspace), NOW, budget).is_none(),
        "an unsupported record is never interpreted"
    );
    let narrative_only = StoredContinuation::Current(SessionContinuationV1 {
        version: CONTINUATION_VERSION,
        source: boundary(),
        narrative: Some(GeneratedNarrative {
            text: "a wrap-up".into(),
            source: NarrativeSource::Summarizer,
        }),
        user_notes: None,
        files: Vec::new(),
        checks: Vec::new(),
    });
    assert!(
        render_record_block_at(Some(&narrative_only), Some(&workspace), NOW, budget).is_none(),
        "a record with no notes, files, or checks renders nothing"
    );
    let content = current_record(Some("a note"), Vec::new(), Vec::new());
    assert!(
        render_record_block_at(Some(&content), Some(&workspace), NOW, 0).is_none(),
        "a zero budget admits no block"
    );
    let _ = std::fs::remove_dir_all(root);
}

/// Notes render under their own user heading, wrapped by the fixed untrusted
/// header, and nothing else joins them when the record has no facts.
#[test]
fn notes_render_under_their_user_heading() {
    let (root, workspace) = workspace("notes");
    let record = current_record(Some("remember the migration note"), Vec::new(), Vec::new());
    let block = render_record_block(Some(&record), Some(&workspace), 16 * 1024)
        .expect("notes alone carry a block");
    assert_eq!(block.label, "continuation-record");
    assert!(!block.truncated);
    assert!(block.body.starts_with(HEADER), "body: {}", block.body);
    assert!(
        block
            .body
            .contains(&format!("{NOTES_HEADING}\nremember the migration note")),
        "body: {}",
        block.body
    );
    assert_eq!(
        block.body.find(NOTES_HEADING),
        Some(HEADER.len() + 1),
        "the notes section follows the header"
    );
    let _ = std::fs::remove_dir_all(root);
}

/// Every file status is computed by refreshing the recorded digest from the
/// bound workspace: unchanged, changed, not found, unreadable, and the
/// digest-absent shape that never reads.
#[test]
fn file_statuses_are_refreshed_from_disk() {
    let (root, workspace) = workspace("statuses");
    std::fs::write(root.join("same.md"), "steady contents\n").unwrap();
    let steady_digest = sha256_of("steady contents\n");
    std::fs::write(root.join("edited.md"), "first draft\n").unwrap();
    let first_draft_digest = sha256_of("first draft\n");
    std::fs::write(root.join("edited.md"), "second draft\n").unwrap();
    std::fs::create_dir_all(root.join("blocked")).unwrap();

    let files = vec![
        file_fact("same.md", Some(steady_digest.clone()), TWO_MINUTES_AGO),
        file_fact("edited.md", Some(first_draft_digest), TWO_MINUTES_AGO),
        file_fact("deleted.md", Some(steady_digest.clone()), TWO_MINUTES_AGO),
        file_fact("blocked", Some(steady_digest.clone()), TWO_MINUTES_AGO),
        file_fact("unknown.md", None, TWO_MINUTES_AGO),
    ];
    let record = current_record(None, files, Vec::new());
    let block = render_record_block_at(Some(&record), Some(&workspace), NOW, 16 * 1024)
        .expect("file facts carry a block");
    assert!(block.body.contains(FILES_HEADING), "body: {}", block.body);
    for line in [
        "- same.md — unchanged since observed (observed 2m ago)",
        "- edited.md — changed since observed (observed 2m ago)",
        "- deleted.md — not found (observed 2m ago)",
        "- blocked — unreadable (observed 2m ago)",
        "- unknown.md — digest not recorded (observed 2m ago)",
    ] {
        assert!(
            block.body.contains(line),
            "missing {line:?} in {}",
            block.body
        );
    }
    let _ = std::fs::remove_dir_all(root);
}

/// The refresh bound: only the first `MAX_REFRESHED_FILES` files in record
/// order are re-read; the rest report `not re-checked` even when their
/// recorded digests would no longer match disk.
#[test]
fn the_refresh_bound_caps_disk_work_at_sixteen_files() {
    let (root, workspace) = workspace("bound");
    let mut files = Vec::new();
    for index in 0..20 {
        let name = format!("file{index}.txt");
        std::fs::write(root.join(&name), format!("contents {index}\n")).unwrap();
        files.push(file_fact(
            &name,
            Some(sha256_of(&format!("contents {index}\n"))),
            TWO_MINUTES_AGO,
        ));
    }
    // The last four files drift on disk: a dropped bound would report them
    // changed, so the count pins the cap, not just the wording.
    for index in 16..20 {
        std::fs::write(root.join(format!("file{index}.txt")), "rewritten\n").unwrap();
    }
    let record = current_record(None, files, Vec::new());
    let block = render_record_block_at(Some(&record), Some(&workspace), NOW, 64 * 1024)
        .expect("twenty file facts carry a block");
    assert_eq!(
        block.body.matches("— unchanged since observed").count(),
        16,
        "exactly the first sixteen are refreshed: {}",
        block.body
    );
    assert_eq!(
        block.body.matches("— not re-checked").count(),
        4,
        "the trailing files are not re-checked: {}",
        block.body
    );
    assert_eq!(
        block.body.matches("— changed since observed").count(),
        0,
        "a drifted file past the bound must not be refreshed: {}",
        block.body
    );
    let _ = std::fs::remove_dir_all(root);
}
