#[test]
fn both_sources_admit_in_fixed_order_with_labels_and_receipts() {
    let (personal, project) = both_files("both");
    let projection = project_instructions(&settings(), Some(&project), &personal, 48 * 1024);
    let second = project_instructions(&settings(), Some(&project), &personal, 48 * 1024);

    assert_eq!(projection.blocks.len(), 2, "one block per admitted source");
    assert_eq!(projection.blocks[0].label, "personal-instructions");
    assert_eq!(
        projection.blocks[1].label,
        "project-instructions (AGENTS.md)"
    );
    for block in &projection.blocks {
        assert!(
            block.body.starts_with(UNTRUSTED_LINE),
            "every instruction block opens with the fixed untrusted framing line"
        );
    }
    assert_eq!(text_of(&projection.blocks[0]), "personal guidance text");
    assert_eq!(text_of(&projection.blocks[1]), "project guidance text");
    assert!(!projection.blocks[0].truncated && !projection.blocks[1].truncated);
    assert_eq!(projection.skipped, Vec::new());
    assert!(projection.omitted.is_empty());
    // Deterministic order and content: unchanged files project equally.
    assert_eq!(projection, second);

    let receipts = &projection.sources;
    assert_eq!(receipts.len(), 2);
    assert_eq!(
        receipts[0].origin,
        saya_harness::instructions::InstructionOrigin::Personal
    );
    assert_eq!(receipts[0].file_name, "instructions.md");
    assert_eq!(
        receipts[0].sha256,
        expected_sha256("personal guidance text")
    );
    assert_eq!(receipts[0].file_size, "personal guidance text".len() as u64);
    assert_eq!(receipts[0].included_bytes, "personal guidance text".len());
    assert_eq!(receipts[0].redactions, 0);
    assert_eq!(
        receipts[1].origin,
        saya_harness::instructions::InstructionOrigin::Project
    );
    assert_eq!(receipts[1].file_name, "AGENTS.md");
    assert_eq!(receipts[1].sha256, expected_sha256("project guidance text"));
    assert_eq!(receipts[1].file_size, "project guidance text".len() as u64);
    let _ = std::fs::remove_dir_all(root_of(&personal));
}

fn root_of(path: &Path) -> PathBuf {
    path.ancestors().nth(2).unwrap().to_path_buf()
}

#[test]
fn disabled_flags_touch_nothing_even_with_files_present() {
    let (personal, project) = both_files("disabled");
    let mut both_off = settings();
    both_off.project_enabled = false;
    both_off.personal_enabled = false;
    let projection = project_instructions(&both_off, Some(&project), &personal, 48 * 1024);
    assert_empty(&projection, "both flags off must load nothing");

    let mut project_off = settings();
    project_off.project_enabled = false;
    let projection = project_instructions(&project_off, Some(&project), &personal, 48 * 1024);
    assert_eq!(projection.blocks.len(), 1);
    assert_eq!(projection.blocks[0].label, "personal-instructions");

    let mut personal_off = settings();
    personal_off.personal_enabled = false;
    let projection = project_instructions(&personal_off, Some(&project), &personal, 48 * 1024);
    assert_eq!(projection.blocks.len(), 1);
    assert_eq!(
        projection.blocks[0].label,
        "project-instructions (AGENTS.md)"
    );
    let _ = std::fs::remove_dir_all(root_of(&personal));
}

fn assert_empty(projection: &InstructionProjection, message: &str) {
    assert!(projection.blocks.is_empty(), "{message}");
    assert!(projection.sources.is_empty(), "{message}");
    assert!(projection.skipped.is_empty(), "{message}");
    assert!(projection.omitted.is_empty(), "{message}");
}

#[test]
fn missing_files_are_silent() {
    let root = stamp("missing");
    let personal = root.join("personal");
    let project = root.join("project");
    std::fs::create_dir_all(&personal).unwrap();
    std::fs::create_dir_all(&project).unwrap();
    let projection = project_instructions(&settings(), Some(&project), &personal, 48 * 1024);
    assert_empty(&projection, "existing dirs without the files are silent");

    let absent = root.join("absent");
    let projection = project_instructions(&settings(), Some(&absent), &absent, 48 * 1024);
    assert_empty(&projection, "absent dirs are silent too");
    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn symlinked_project_source_is_skipped_and_outside_never_leaks() {
    use std::os::unix::fs::symlink;

    let root = stamp("symlink");
    let personal = root.join("personal");
    let project = root.join("project");
    let outside = root.join("outside-agents.md");
    write_file(&outside, "OUTSIDE-SENTINEL secret project rules");
    write_file(&personal.join("instructions.md"), "personal guidance text");
    std::fs::create_dir_all(&project).unwrap();
    symlink(&outside, project.join("AGENTS.md")).unwrap();

    let mut settings = settings();
    settings.personal_enabled = false;
    let projection = project_instructions(&settings, Some(&project), &personal, 48 * 1024);

    assert!(
        projection.blocks.is_empty(),
        "a refused symlink admits no block"
    );
    assert_eq!(projection.sources, Vec::new());
    assert_eq!(projection.skipped.len(), 1);
    assert_eq!(
        projection.skipped[0].origin,
        saya_harness::instructions::InstructionOrigin::Project
    );
    assert!(matches!(
        projection.skipped[0].reason,
        saya_harness::instructions::SkipReason::SymlinkRefused
    ));
    for block in &projection.blocks {
        assert!(
            !block.body.contains("OUTSIDE-SENTINEL"),
            "the symlink target's body must never reach a block: {}",
            block.body
        );
    }
    let _ = std::fs::remove_dir_all(&root);
}
