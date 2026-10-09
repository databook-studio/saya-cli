/// The data-dir guard (T2): the investigation fixtures must never touch the
/// machine's real data directory. The default root — computed the production
/// way (`SAYA_INVESTIGATIONS_DIR` when set, else beside the state DB) — must
/// gain no files while the investigation fixtures run through the real
/// dispatch, every fixture's composed root must sit under the temp dir, and
/// the save must have landed in that composed root (a positive control: the
/// fixtures really write, just somewhere private).
#[test]
fn tests_never_touch_the_real_data_dir() {
    let default_root = crate::config::runtime::investigations_root_from(
        std::env::var_os("SAYA_INVESTIGATIONS_DIR").as_deref(),
        &crate::state_path::state_db_path(),
    );
    let before = document_count(&default_root);

    // The chain fixture's composed root, plus the roots the two shared App
    // fixtures compose: all of them must be temp-rooted.
    let fixture_roots = [
        (
            "chain fixture",
            ChainFixture::build("data-dir-guard")
                .runtime
                .investigations_root,
        ),
        ("idle app", idle_app().runtime.investigations_root.clone()),
        ("unused runtime", unused_runtime().investigations_root),
    ];
    for (name, root) in &fixture_roots {
        assert!(
            root.starts_with(std::env::temp_dir()),
            "the {name} composed its investigations root at {root:?}, outside the temp dir"
        );
    }

    let mut fx = ChainFixture::build("data-dir-guard-writes");
    fx.app.last_query = Some(LastQuery {
        sql: "SELECT 1".to_string(),
        connection: Some("demo".to_string()),
    });
    let outcome = dispatch_line("/investigation save data-dir-guard-sentinel", &mut fx);
    assert!(
        matches!(outcome, Dispatch::Handled),
        "the save dispatches inline"
    );
    let outcome = dispatch_line("/investigation list", &mut fx);
    assert!(
        matches!(outcome, Dispatch::Handled),
        "the listing dispatches inline"
    );
    let outcome = dispatch_line("/investigation run no-such-ffffffff", &mut fx);
    assert!(
        matches!(outcome, Dispatch::ReplayTask(_)),
        "the run dispatches its task without running it"
    );
    assert!(
        document_count(&fx.runtime.investigations_root) > 0,
        "the save really wrote into the fixture's composed root"
    );

    let after = document_count(&default_root);
    assert_eq!(
        before, after,
        "the investigations root {default_root:?} gained files during the fixtures"
    );
}

/// Entries in a directory, or 0 when it does not exist — a read-only count,
/// never a creation.
fn document_count(root: &std::path::Path) -> usize {
    std::fs::read_dir(root)
        .map(|entries| entries.count())
        .unwrap_or(0)
}

/// `/report` writes the Markdown report from the capture through the real
/// dispatch: no query, no task, the file on disk.
#[test]
fn report_through_dispatch_writes() {
    let mut fx = ChainFixture::build("report");
    fx.app.captured = captured_of(sample_result());
    let path = fx.root.0.join("report.md");
    let outcome = dispatch_line(&format!("/report --rows 2 {}", path.display()), &mut fx);
    assert!(
        matches!(outcome, Dispatch::Handled),
        "the report is handled inline"
    );
    let written = std::fs::read_to_string(&path).expect("the report was written");
    assert!(
        written.contains("sentinel-alice"),
        "the rows come from the capture: {written}"
    );
    let msg = last_block(&fx.app.transcript, BlockKind::System).expect("success is said");
    assert!(
        msg.contains("Wrote report to") && msg.contains("(2 rows included)"),
        "the success message counts the rows: {msg}"
    );
}

/// `/context` through the real dispatch: the workspace-root AGENTS.md is
/// named with its whole-file size and truncation flag, and the body text is
/// never printed — the view shows receipts, not contents.
#[test]
fn context_dispatch_lists_the_loaded_source_and_never_its_body() {
    let mut fx = ChainFixture::build("context-lists");
    let body = format!("guidance with {CONTEXT_SENTINEL} inside\n");
    std::fs::write(fx.root.0.join("AGENTS.md"), &body).unwrap();
    assert!(
        matches!(dispatch_line("/context", &mut fx), Dispatch::Handled),
        "/context is handled by the dispatch chain"
    );
    let view =
        last_block(&fx.app.transcript, BlockKind::System).expect("/context pushes a system block");
    assert!(
        view.contains("AGENTS.md"),
        "the loaded source is named: {view}"
    );
    assert!(
        view.contains(&format!("{} B file", body.len())),
        "the whole-file size is shown: {view}"
    );
    assert!(
        view.contains("truncated: no"),
        "the truncation flag is shown: {view}"
    );
    assert!(
        view.contains("never permissions"),
        "the guidance-never-permissions note is stated: {view}"
    );
    assert!(
        !view.contains(CONTEXT_SENTINEL),
        "the source body must never appear: {view}"
    );
}
