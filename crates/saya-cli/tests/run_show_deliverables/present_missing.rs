/// A completed run's declared deliverables are listed with their size and
/// digest.
#[test]
fn a_completed_run_shows_its_deliverables_with_sizes_and_digests() {
    let env = test_root("delivered");
    let id = run_writing(
        &env,
        &[("report.md", "the survey report")],
        "report.md",
        REPORT_CONTENT,
        "produce the survey report",
    );
    let (address, _ready) = mock(Vec::new());
    let show = show(&env, &id, &address);
    assert_eq!(show.status.code(), Some(0), "stderr: {}", stderr(&show));
    let shown = stdout(&show);
    assert!(
        shown.contains("report.md"),
        "the deliverable's name must be listed: {shown}"
    );
    assert!(
        shown.contains(&format!("{} bytes", REPORT_CONTENT.len())),
        "the deliverable's size must be listed: {shown}"
    );
    assert!(
        shown.contains(REPORT_DIGEST),
        "the deliverable's digest must be listed: {shown}"
    );
    // The manifest is durable: the journal carries it, digest included.
    let journal = fs::read_to_string(env.runs.join(&id).join("events.ndjson")).unwrap();
    assert!(
        journal.contains("deliverables") && journal.contains(REPORT_DIGEST),
        "the journal must record the manifest: {journal}"
    );
    let _ = fs::remove_dir_all(&env.root);
}

/// A declared deliverable the step never produced is reported missing,
/// distinctly from one that exists — a run may not look complete while its
/// declared outputs are absent.
#[test]
fn a_declared_deliverable_the_step_never_produced_is_reported_missing() {
    let env = test_root("missing");
    let id = run_writing_nothing(
        &env,
        &[("report.md", "the survey report")],
        "produce the survey report",
    );
    let (address, _ready) = mock(Vec::new());
    let show = show(&env, &id, &address);
    assert_eq!(show.status.code(), Some(0), "stderr: {}", stderr(&show));
    let shown = stdout(&show);
    assert!(
        shown.contains("report.md missing"),
        "the declared-but-absent deliverable must be reported missing: {shown}"
    );
    assert!(
        !shown.contains("sha256"),
        "a missing deliverable must not carry a digest: {shown}"
    );
    let _ = fs::remove_dir_all(&env.root);
}
