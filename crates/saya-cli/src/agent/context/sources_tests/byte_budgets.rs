#[test]
fn oversize_source_truncates_at_a_utf8_boundary_with_full_receipt() {
    let root = stamp("oversize");
    let personal = root.join("personal");
    let project = root.join("project");
    // Multibyte characters make a mid-character cap cut possible.
    let contents = "héllo wörld — ünïcode — ".repeat(20);
    write_file(&project.join("AGENTS.md"), &contents);

    let mut settings = settings();
    settings.personal_enabled = false;
    settings.project_max_bytes = 64;
    let projection = project_instructions(&settings, Some(&project), &personal, 48 * 1024);

    assert_eq!(projection.blocks.len(), 1);
    let block = &projection.blocks[0];
    assert!(block.truncated, "a capped source is marked truncated");
    let text = text_of(block);
    assert!(
        contents.starts_with(text),
        "the admitted text is a byte prefix of the source: {text:?}"
    );
    assert!(text.len() <= 64, "the cap bounds the admitted text");
    assert!(text.len() < contents.len());
    let receipt = &projection.sources[0];
    assert_eq!(
        receipt.file_size,
        contents.len() as u64,
        "receipt carries the whole file size"
    );
    assert!(receipt.truncated);
    assert_eq!(receipt.sha256, expected_sha256(&contents));
    assert_eq!(receipt.included_bytes, text.len());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn tiny_budget_omits_and_moderate_budget_trims_within_turn_bytes() {
    let root = stamp("budget");
    let personal = root.join("personal");
    let project = root.join("project");
    let contents = "x".repeat(12 * 1024);
    write_file(&project.join("AGENTS.md"), &contents);

    let mut settings = settings();
    settings.personal_enabled = false;

    // A budget too small for a useful stub drops the block entirely.
    let projection = project_instructions(&settings, Some(&project), &personal, 600);
    assert!(
        projection.blocks.is_empty(),
        "a 600-byte budget cannot hold a useful block"
    );
    assert_eq!(projection.sources, Vec::new());
    assert_eq!(projection.omitted.len(), 1);
    assert!(matches!(
        projection.omitted[0].reason,
        OmittedReason::NoBudget
    ));
    assert_eq!(
        projection.omitted[0].origin,
        saya_harness::instructions::InstructionOrigin::Project
    );

    // A moderate budget keeps a trimmed block that the real turn_bytes bounds.
    let remaining = 5_000;
    let projection = project_instructions(&settings, Some(&project), &personal, remaining);
    assert_eq!(projection.blocks.len(), 1);
    let block = &projection.blocks[0];
    assert!(
        block.truncated,
        "a budget-trimmed block is marked truncated"
    );
    assert!(
        text_of(block).len() < contents.len(),
        "the budget trimmed the body"
    );
    let measured = turn_bytes(None, &projection.blocks, "p");
    assert!(
        measured <= remaining + wrapper_allowance(),
        "the fitted blocks must fit the remaining budget through the real turn_bytes: {measured}"
    );
    assert_eq!(projection.sources[0].included_bytes, text_of(block).len());
    assert_eq!(projection.sources[0].file_size, contents.len() as u64);
    // Deterministic under the fit too.
    let second = project_instructions(&settings, Some(&project), &personal, remaining);
    assert_eq!(projection, second);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn aggregate_cap_holds_when_personal_max_exceeds_total() {
    let root = stamp("aggregate");
    let personal = root.join("personal");
    let project = root.join("project");
    write_file(&personal.join("instructions.md"), &"p".repeat(3 * 1024));
    write_file(&project.join("AGENTS.md"), &"q".repeat(3 * 1024));

    let mut settings = settings();
    settings.personal_max_bytes = 4096;
    settings.total_max_bytes = 1024;
    let projection = project_instructions(&settings, Some(&project), &personal, 48 * 1024);

    let total: usize = projection.sources.iter().map(|s| s.included_bytes).sum();
    assert!(
        total <= settings.total_max_bytes,
        "the aggregate invariant caps both sources together: {total}"
    );
    let bodies: usize = projection.blocks.iter().map(|b| text_of(b).len()).sum();
    assert!(bodies <= settings.total_max_bytes);
    let _ = std::fs::remove_dir_all(&root);
}
