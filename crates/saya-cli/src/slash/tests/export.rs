#[test]
fn test_parse_export_command() {
    assert_eq!(
        parse_slash_command("/export out.csv"),
        Ok(Some(SlashCommand::Export(export_request(
            None, false, "out.csv"
        ))))
    );
    assert_eq!(
        parse_slash_command("/export"),
        Err(SlashParseError(
            "export requires a file path, e.g. /export out.csv".into()
        ))
    );
}

/// `/export --snapshot|--refresh` choose what is written; the legacy
/// form (no mode flag) stays a refresh. Flags may appear in any order
/// before the path, and `--overwrite` composes with either mode.
#[test]
fn export_mode_flags_parse_in_any_order() {
    assert_eq!(
        parse_slash_command("/export --snapshot out.csv"),
        Ok(Some(SlashCommand::Export(export_request(
            Some(ExportMode::Snapshot),
            false,
            "out.csv"
        ))))
    );
    assert_eq!(
        parse_slash_command("/export --refresh out.csv"),
        Ok(Some(SlashCommand::Export(export_request(
            Some(ExportMode::Refresh),
            false,
            "out.csv"
        ))))
    );
    assert_eq!(
        parse_slash_command("/export --overwrite out.csv"),
        Ok(Some(SlashCommand::Export(export_request(
            None, true, "out.csv"
        ))))
    );
    // Any order before the path.
    assert_eq!(
        parse_slash_command("/export --overwrite --snapshot out.csv"),
        Ok(Some(SlashCommand::Export(export_request(
            Some(ExportMode::Snapshot),
            true,
            "out.csv"
        ))))
    );
    assert_eq!(
        parse_slash_command("/export --refresh --overwrite out.csv"),
        Ok(Some(SlashCommand::Export(export_request(
            Some(ExportMode::Refresh),
            true,
            "out.csv"
        ))))
    );
}

/// Both modes together, an unknown `--flag`, and a flag with no path are
/// usage errors — never silently read as path text.
#[test]
fn export_flag_mistakes_are_usage_errors() {
    let both = parse_slash_command("/export --snapshot --refresh out.csv").unwrap_err();
    assert!(
        both.0.contains("--snapshot or --refresh, not both") && both.0.contains("usage"),
        "both modes together must be refused with usage: {both}"
    );
    let unknown = parse_slash_command("/export --bogus out.csv").unwrap_err();
    assert!(
        unknown.0.contains("unknown export flag: --bogus") && unknown.0.contains("usage"),
        "an unknown flag must be a usage error: {unknown}"
    );
    assert_eq!(
        parse_slash_command("/export --snapshot"),
        Err(SlashParseError(
            "export requires a file path, e.g. /export out.csv".into()
        ))
    );
    assert_eq!(
        parse_slash_command("/export --snapshot --overwrite"),
        Err(SlashParseError(
            "export requires a file path, e.g. /export out.csv".into()
        ))
    );
}

/// The path is everything after the flags, whitespace included — the
/// pre-flags grammar let paths contain spaces and that must survive.
#[test]
fn export_paths_may_contain_spaces() {
    assert_eq!(
        parse_slash_command("/export my file.csv"),
        Ok(Some(SlashCommand::Export(export_request(
            None,
            false,
            "my file.csv"
        ))))
    );
    assert_eq!(
        parse_slash_command("/export --snapshot --overwrite my file.csv"),
        Ok(Some(SlashCommand::Export(export_request(
            Some(ExportMode::Snapshot),
            true,
            "my file.csv"
        ))))
    );
}

fn report_request(path: &str, rows: Option<usize>, overwrite: bool) -> SlashCommand {
    SlashCommand::Report(ReportRequest {
        path: path.into(),
        rows,
        overwrite,
    })
}
