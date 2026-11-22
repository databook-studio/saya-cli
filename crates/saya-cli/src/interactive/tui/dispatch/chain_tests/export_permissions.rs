/// The line that reports the fixture's capture: `/export --format md`, and
/// the deprecated `/report` spelling of it, write the same document.
#[test]
fn export_md_writes_todays_report_bytes() {
    let mut fx = ChainFixture::build("export-md");
    fx.app.captured = captured_of(sample_result());
    let path = fx.root.0.join("export.md");
    let outcome = dispatch_line(&format!("/export --rows 2 {}", path.display()), &mut fx);
    assert!(
        matches!(outcome, Dispatch::Handled),
        "the report form is handled inline, with no task"
    );
    let written = std::fs::read_to_string(&path).expect("the report was written");
    // Today's report, pinned by its own writer into a second path: the two
    // documents agree byte for byte except the line naming when each ran.
    let expected_path = fx.root.0.join("expected.md");
    let capture = fx.app.captured.as_ref().expect("the fixture captures");
    crate::interactive::tui::export::write_report(
        &capture.result,
        &capture.evidence,
        Some(2),
        &expected_path,
        false,
    )
    .expect("the writer writes the fixture report");
    let expected = std::fs::read_to_string(&expected_path).expect("the report was written");
    assert_eq!(
        without_generated_line(&written),
        without_generated_line(&expected),
        "a .md destination is today's /report document, byte for byte"
    );
    for pinned in [
        "# saya report",
        "sentinel-alice",
        "SELECT id, name FROM users",
        "## Provenance",
        "Submitted SQL sha256:",
    ] {
        assert!(written.contains(pinned), "{pinned} is in {written}");
    }
    let msg = last_block(&fx.app.transcript, BlockKind::System).expect("success is said");
    assert!(
        msg.contains("Wrote report to") && msg.contains("(2 rows included)"),
        "the success message counts the rows: {msg}"
    );
    assert!(
        !msg.contains("note:"),
        "the canonical spelling states no alias note: {msg}"
    );
}

/// `/report x.md` is `/export x.md --format md`: the same bytes, plus the
/// note the deprecated spelling says once.
#[test]
fn report_is_export_format_md_plus_its_note() {
    let mut fx = ChainFixture::build("report-alias");
    fx.app.captured = captured_of(sample_result());
    let alias_path = fx.root.0.join("report.md");
    let canonical_path = fx.root.0.join("canonical.md");
    let outcome = dispatch_line(&format!("/report --rows 2 {}", alias_path.display()), &mut fx);
    assert!(matches!(outcome, Dispatch::Handled));
    let alias_msg = last_block(&fx.app.transcript, BlockKind::System).expect("success is said");
    assert!(
        alias_msg.contains("note: /report is now /export --format md"),
        "the alias states its note: {alias_msg}"
    );
    dispatch_line(
        &format!("/export --format md --rows 2 {}", canonical_path.display()),
        &mut fx,
    );
    let canonical_msg = last_block(&fx.app.transcript, BlockKind::System).expect("success is said");
    assert!(
        !canonical_msg.contains("note:"),
        "the note was already said this session, and never rides the canonical spelling: \
         {canonical_msg}"
    );
    let alias = std::fs::read_to_string(&alias_path).expect("the alias wrote the report");
    let canonical = std::fs::read_to_string(&canonical_path).expect("the export wrote the report");
    assert_eq!(
        without_generated_line(&alias),
        without_generated_line(&canonical),
        "/report wrote the bytes /export --format md writes"
    );
}

/// `/export out.csv` keeps today's data-file bytes: the snapshot of the
/// capture, exactly as the data writer writes it.
#[test]
fn export_csv_bytes_are_unchanged() {
    let mut fx = ChainFixture::build("export-csv");
    fx.app.captured = captured_of(sample_result());
    let path = fx.root.0.join("export.csv");
    let outcome = dispatch_line(&format!("/export --snapshot {}", path.display()), &mut fx);
    assert!(matches!(outcome, Dispatch::Handled));
    let written = std::fs::read_to_string(&path).expect("the csv was written");
    let expected_path = fx.root.0.join("expected.csv");
    let capture = fx.app.captured.as_ref().expect("the fixture captures");
    crate::interactive::tui::export::write_result_overwrite(&capture.result, &expected_path, false)
        .expect("the writer writes the fixture csv");
    let expected = std::fs::read_to_string(&expected_path).expect("the csv was written");
    assert_eq!(written, expected, "a .csv destination is the data file");
    assert!(
        written.contains("sentinel-alice") && !written.contains("## Provenance"),
        "the data file is rows, not the report: {written}"
    );
}

/// `/permissions` states the five facts; `/grants` is its alias — the same
/// view, plus its note once.
#[test]
fn permissions_states_the_five_facts_and_grants_is_its_alias() {
    let mut fx = ChainFixture::build("permissions");
    assert!(
        matches!(dispatch_line("/permissions", &mut fx), Dispatch::Handled),
        "/permissions is handled inline"
    );
    let view = last_block(&fx.app.transcript, BlockKind::System).expect("the view is a block");
    for label in ["Mode:", "Grants:", "Deny list:", "Host lane:", "Data sharing:"] {
        assert!(view.contains(label), "{label} is stated: {view}");
    }
    assert!(
        !view.contains("note:"),
        "the canonical spelling states no alias note: {view}"
    );
    let canonical = view;

    assert!(matches!(
        dispatch_line("/grants", &mut fx),
        Dispatch::Handled
    ));
    let alias = last_block(&fx.app.transcript, BlockKind::System).expect("the alias answers");
    assert_eq!(
        alias,
        format!("{canonical}\n{}", crate::chat::ops_status::Note::Grants.text()),
        "the alias answers the one view, plus its note"
    );
    dispatch_line("/grants", &mut fx);
    let again = last_block(&fx.app.transcript, BlockKind::System).expect("the alias answers");
    assert_eq!(again, canonical, "the note is said once per session");
}

/// `/investigations` is `/investigation list`: one command, and the same
/// text rendering through the chain. Row 156: the bare spelling opens the
/// TUI's picker, so the text listing is asked for with `--list`.
#[test]
fn investigations_is_the_investigation_listing() {
    let mut alias = ChainFixture::build("investigations-alias");
    assert!(
        matches!(dispatch_line("/investigations", &mut alias), Dispatch::OpenInvestigationPicker),
        "bare /investigations opens the picker in the TUI"
    );
    assert!(matches!(
        dispatch_line("/investigations --list", &mut alias),
        Dispatch::Handled
    ));
    let alias_view =
        last_block(&alias.app.transcript, BlockKind::System).expect("the listing is a block");
    let mut canonical = ChainFixture::build("investigation-list");
    assert!(
        matches!(
            dispatch_line("/investigation", &mut canonical),
            Dispatch::OpenInvestigationPicker
        ),
        "and so does bare /investigation"
    );
    assert!(matches!(
        dispatch_line("/investigation list", &mut canonical),
        Dispatch::Handled
    ));
    let canonical_view =
        last_block(&canonical.app.transcript, BlockKind::System).expect("the listing is a block");
    assert_eq!(
        alias_view, canonical_view,
        "/investigations --list renders what /investigation list renders"
    );
}

/// The one line of the report that names when it ran: byte-identity is
/// asserted on the rest, because two runs can straddle a second.
fn without_generated_line(document: &str) -> String {
    document
        .lines()
        .filter(|line| !line.starts_with("Generated "))
        .collect::<Vec<_>>()
        .join("\n")
}
