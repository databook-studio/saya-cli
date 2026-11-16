#[test]
fn test_parse_report_command() {
    assert_eq!(
        parse_slash_command("/report notes.md"),
        Ok(Some(report_request("notes.md", None, false)))
    );
    assert_eq!(
        parse_slash_command("/report --rows 20 notes.md"),
        Ok(Some(report_request("notes.md", Some(20), false)))
    );
    assert_eq!(
        parse_slash_command("/report --overwrite notes.md"),
        Ok(Some(report_request("notes.md", None, true)))
    );
    // Flags in any order before the path; the path stays verbatim.
    assert_eq!(
        parse_slash_command("/report --rows 20 --overwrite my notes.md"),
        Ok(Some(report_request("my notes.md", Some(20), true)))
    );
    assert_eq!(
        parse_slash_command("/report --overwrite --rows 20 notes.md"),
        Ok(Some(report_request("notes.md", Some(20), true)))
    );
    // A bare /report and a flagless tail are usage errors.
    assert_eq!(
        parse_slash_command("/report"),
        Err(SlashParseError(
            "report requires a file path, e.g. /report out.md".into()
        ))
    );
    assert_eq!(
        parse_slash_command("/report --rows 20"),
        Err(SlashParseError(
            "report requires a file path, e.g. /report out.md".into()
        ))
    );
}

/// `--rows` is capped at the shared 100-row bound; the bound and below
/// parse, above it is a usage error naming the cap.
#[test]
fn report_rows_bound_is_a_usage_error() {
    assert_eq!(
        parse_slash_command("/report --rows 100 out.md"),
        Ok(Some(report_request("out.md", Some(100), false)))
    );
    assert_eq!(
        parse_slash_command("/report --rows 0 out.md"),
        Ok(Some(report_request("out.md", Some(0), false)))
    );
    let error = parse_slash_command("/report --rows 101 out.md").unwrap_err();
    assert!(
        error.0.contains("capped at 100") && error.0.contains("usage"),
        "the cap is named with usage: {error}"
    );
}

/// An unknown `--flag`, `--rows` twice, `--rows` without a number, and a
/// number-shaped value are usage errors — never read as path text.
#[test]
fn report_flag_mistakes_are_usage_errors() {
    let unknown = parse_slash_command("/report --bogus out.md").unwrap_err();
    assert!(
        unknown.0.contains("unknown report flag: --bogus") && unknown.0.contains("usage"),
        "an unknown flag must be a usage error: {unknown}"
    );
    let twice = parse_slash_command("/report --rows 5 --rows 6 out.md").unwrap_err();
    assert!(
        twice.0.contains("--rows given twice"),
        "a repeated --rows is refused: {twice}"
    );
    let no_value = parse_slash_command("/report --rows").unwrap_err();
    assert!(
        no_value.0.contains("--rows needs a number"),
        "a valueless --rows is refused: {no_value}"
    );
    let not_number = parse_slash_command("/report --rows abc out.md").unwrap_err();
    assert!(
        not_number.0.contains("--rows needs a number"),
        "a non-numeric --rows is refused: {not_number}"
    );
}

/// `.csv`/`.json` destinations get the usage hint pointing at `/export`;
/// any other extension is fine — `.md` is conventional, not enforced.
#[test]
fn report_data_file_extension_gets_a_usage_hint() {
    for path in ["out.csv", "out.json"] {
        let error = parse_slash_command(&format!("/report {path}")).unwrap_err();
        assert!(
            error.0.contains("use /export for data files"),
            "the hint names /export: {error}"
        );
    }
    assert_eq!(
        parse_slash_command("/report out.txt"),
        Ok(Some(report_request("out.txt", None, false)))
    );
    assert_eq!(
        parse_slash_command("/report out"),
        Ok(Some(report_request("out", None, false)))
    );
}
