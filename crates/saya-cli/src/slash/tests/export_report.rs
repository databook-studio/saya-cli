// K7: `/export` absorbs `/report`. The extension or `--format` selects the
// file; `.md` (or `--format md`) writes today's report bytes, and `/report`
// is that form's one-release alias — one variant, one request, one writer.

/// `/export out.md` selects the Markdown report by extension, and the
/// `.csv`/`.json` forms are unchanged requests.
#[test]
fn export_selects_the_format_from_the_extension() {
    let markdown = parse_slash_command("/export out.md").unwrap().unwrap();
    let SlashCommand::Export(request) = markdown else {
        panic!("/export parses to the one Export variant: {markdown:?}");
    };
    assert_eq!(request.path, "out.md");
    assert_eq!(
        request.format,
        None,
        "no --format was stated: the extension selects"
    );
    assert_eq!(
        request.format(),
        Some(ExportFormat::Markdown),
        "a .md destination is the report"
    );
    assert!(request.is_report());
    assert!(!request.via_report_alias, "the canonical spelling states it");

    let csv = parse_slash_command("/export out.csv").unwrap().unwrap();
    let SlashCommand::Export(request) = csv else {
        panic!("/export parses to the one Export variant");
    };
    assert_eq!(request.format(), Some(ExportFormat::Csv));
    assert!(!request.is_report());
    let spelled = parse_slash_command("/export OUT.JSON").unwrap().unwrap();
    let SlashCommand::Export(request) = spelled else {
        panic!("/export parses to the one Export variant");
    };
    assert_eq!(
        request.format(),
        Some(ExportFormat::Json),
        "the extension match is case-insensitive"
    );
}

/// `--format` states the file, whatever the destination is called; `--rows`
/// bounds the report's table.
#[test]
fn the_format_flag_selects_and_rows_bounds_the_table() {
    for (text, format) in [
        ("/export --format md out.txt", ExportFormat::Markdown),
        ("/export --format csv out.txt", ExportFormat::Csv),
        ("/export --format json out.txt", ExportFormat::Json),
        ("/export --format md out.md", ExportFormat::Markdown),
    ] {
        let parsed = parse_slash_command(text).unwrap().unwrap();
        let SlashCommand::Export(request) = parsed else {
            panic!("{text} parses to the one Export variant");
        };
        assert_eq!(request.format(), Some(format), "{text}");
    }
    let parsed = parse_slash_command("/export --rows 20 --format md notes.md")
        .unwrap()
        .unwrap();
    let SlashCommand::Export(request) = parsed else {
        panic!("the flags parse in any order before the path");
    };
    assert_eq!(request.rows, Some(20));
    assert_eq!(request.path, "notes.md");
    assert!(request.is_report());
}

/// `/report <args>` is `/export <args> --format md`, plus the alias flag: the
/// same path, the same rows, the same overwrite, the same resolved format —
/// one operation.
#[test]
fn report_is_export_format_md_with_its_note() {
    let alias = parse_slash_command("/report --rows 20 --overwrite my notes.md")
        .unwrap()
        .unwrap();
    let canonical = parse_slash_command("/export --rows 20 --overwrite my notes.md")
        .unwrap()
        .unwrap();
    let (SlashCommand::Export(alias), SlashCommand::Export(canonical)) = (alias, canonical)
    else {
        panic!("/report and /export are one variant");
    };
    assert!(
        alias.via_report_alias,
        "/report says which spelling asked for it"
    );
    assert_eq!(
        alias.format,
        Some(ExportFormat::Markdown),
        "the alias states the format the report always was"
    );
    assert_eq!(
        alias.format(),
        canonical.format(),
        "same format: the alias states it, the canonical spelling lets the extension select"
    );
    assert_eq!(alias.path, canonical.path);
    assert_eq!(alias.rows, canonical.rows);
    assert_eq!(alias.overwrite, canonical.overwrite);
    assert_eq!(alias.mode, canonical.mode, "neither states a mode");
}

/// The alias keeps today's refusals: a data-file destination and a foreign
/// `--format` both point at `/export`, and the format mistakes are usage
/// errors, never path text.
#[test]
fn report_refuses_data_destinations_and_foreign_formats() {
    for path in ["out.csv", "out.json"] {
        let error = parse_slash_command(&format!("/report {path}")).unwrap_err();
        assert!(
            error.0.contains("use /export for data files"),
            "the hint names /export: {error}"
        );
    }
    let error = parse_slash_command("/report --format csv out.txt").unwrap_err();
    assert!(
        error.0.contains("a report is Markdown") && error.0.contains("--format csv"),
        "a report cannot be CSV: {error}"
    );
    // `/report --format md` states what it already is, and parses.
    assert_eq!(
        parse_slash_command("/report --format md out.md").unwrap(),
        Some(SlashCommand::Export(ExportRequest {
            format: Some(ExportFormat::Markdown),
            path: "out.md".into(),
            via_report_alias: true,
            ..ExportRequest::default()
        }))
    );
}

/// The format and rows grammars refuse their own mistakes, and the report's
/// `--rows` bound is the one shared cap.
#[test]
fn format_and_rows_mistakes_are_usage_errors() {
    let unknown = parse_slash_command("/export --format yaml out.txt").unwrap_err();
    assert!(
        unknown.0.contains("--format csv|json|md, got: yaml") && unknown.0.contains("usage"),
        "an unknown format is named with usage: {unknown}"
    );
    let valueless = parse_slash_command("/export --format").unwrap_err();
    assert!(
        valueless.0.contains("--format needs a value"),
        "a valueless --format is refused: {valueless}"
    );
    let twice = parse_slash_command("/export --format md --format csv out.md").unwrap_err();
    assert!(
        twice.0.contains("--format given twice"),
        "a repeated --format is refused: {twice}"
    );
    let capped = parse_slash_command("/export --rows 101 out.md").unwrap_err();
    assert!(
        capped.0.contains("capped at 100"),
        "the one shared row cap binds /export too: {capped}"
    );
    let unknown_rows = parse_slash_command("/export --rows five out.md").unwrap_err();
    assert!(
        unknown_rows.0.contains("--rows needs a number"),
        "a non-numeric --rows is refused: {unknown_rows}"
    );
    // The report spelling has no modes to state: they stay unknown flags.
    let mode = parse_slash_command("/report --snapshot out.md").unwrap_err();
    assert!(
        mode.0.contains("unknown report flag: --snapshot"),
        "the report spelling refuses the modes: {mode}"
    );
    let missing = parse_slash_command("/export --format md").unwrap_err();
    assert_eq!(
        missing.0, "export requires a file path, e.g. /export out.csv",
        "a flagged tail with no path is the pinned usage error"
    );
}
