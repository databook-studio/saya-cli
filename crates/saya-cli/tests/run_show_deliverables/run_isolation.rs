/// Two runs' deliverables do not bleed: `show` on run A never lists run B's
/// files, and the other way round.
#[test]
fn two_runs_deliverables_do_not_bleed() {
    let env = test_root("no-bleed");
    let id_b = run_writing(
        &env,
        &[("b-notes.md", "run b's notes")],
        "b-notes.md",
        B_CONTENT,
        "run b produces its notes",
    );
    let id_a = run_writing(
        &env,
        &[("a-report.md", "run a's report")],
        "a-report.md",
        A_CONTENT,
        "run a produces its report",
    );
    assert_ne!(id_a, id_b, "the two runs must be distinct");
    let (address, _ready) = mock(Vec::new());
    let (id_a, id_b) = if goal_of(&env, &id_a).contains("run a") {
        (id_a, id_b)
    } else {
        (id_b, id_a)
    };
    let shown_a = stdout(&show(&env, &id_a, &address));
    assert!(
        shown_a.contains("a-report.md") && shown_a.contains(A_DIGEST),
        "run A's deliverable must be listed: {shown_a}"
    );
    assert!(
        !shown_a.contains("b-notes.md") && !shown_a.contains(B_DIGEST),
        "run B's files must never appear in run A's show: {shown_a}"
    );
    let shown_b = stdout(&show(&env, &id_b, &address));
    assert!(
        shown_b.contains("b-notes.md") && shown_b.contains(B_DIGEST),
        "run B's deliverable must be listed: {shown_b}"
    );
    assert!(
        !shown_b.contains("a-report.md") && !shown_b.contains(A_DIGEST),
        "run A's files must never appear in run B's show: {shown_b}"
    );
    let _ = fs::remove_dir_all(&env.root);
}

/// The recorded digest matches the file's content at completion; corrupting
/// the file afterwards changes nothing — the record is what completed, not
/// what is there now.
#[test]
fn the_recorded_digest_is_what_completed_and_survives_later_corruption() {
    let env = test_root("digest");
    let id = run_writing(
        &env,
        &[("report.md", "the survey report")],
        "report.md",
        REPORT_CONTENT,
        "produce the survey report",
    );
    let (address, _ready) = mock(Vec::new());
    let shown = stdout(&show(&env, &id, &address));
    assert!(
        shown.contains(REPORT_DIGEST),
        "the recorded digest must match the completed content: {shown}"
    );
    // Corrupt the workspace file after completion.
    let artifact = env.runs.join(&id).join("workspace").join("report.md");
    fs::write(&artifact, "tampered after completion\n").unwrap();
    let shown_after = stdout(&show(&env, &id, &address));
    assert!(
        shown_after.contains(REPORT_DIGEST),
        "the recorded digest must be unchanged by later corruption: {shown_after}"
    );
    assert!(
        shown_after.contains(&format!("{} bytes", REPORT_CONTENT.len())),
        "the recorded size must be unchanged by later corruption: {shown_after}"
    );
    let _ = fs::remove_dir_all(&env.root);
}
