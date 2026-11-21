/// Parses the `/export` tail: leading flags in any order, then the path —
/// the rest of the tail verbatim. The extension or `--format csv|json|md`
/// selects the file; `.md` (or `--format md`) writes today's Markdown report.
/// Unknown `--flag`s, both modes together, an unknown or repeated `--format`,
/// a bad `--rows`, and a missing path are usage errors, so a flag is never
/// read as path text and a path is never read as a flag.
fn parse_export(tail: &str) -> Result<ExportRequest, SlashParseError> {
    parse_export_tail(tail, false)
}

/// Parses the `/report` tail — the one-release alias of `/export … --format
/// md`: the same grammar, the format fixed to Markdown, and the alias
/// recorded so the answer carries its note. A `.csv`/`.json` destination
/// keeps today's hint that data files belong to `/export`, and a `--format`
/// naming another format is refused with the same pointer: a report is
/// Markdown, one document, whatever spelling asked for it.
fn parse_report(tail: &str) -> Result<ExportRequest, SlashParseError> {
    parse_export_tail(tail, true)
}

fn parse_export_tail(tail: &str, report: bool) -> Result<ExportRequest, SlashParseError> {
    let usage = if report {
        " (usage: /report [--rows N] [--overwrite] <path>)"
    } else {
        " (usage: /export [--snapshot|--refresh] [--overwrite] [--rows N] \
         [--format csv|json|md] <path>)"
    };
    let spelling = if report { "report" } else { "export" };
    let mut mode: Option<ExportMode> = None;
    let mut overwrite = false;
    let mut rows: Option<usize> = None;
    let mut format: Option<ExportFormat> = None;
    let mut rest = tail;
    loop {
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let (token, remainder) = (&rest[..end], rest[end..].trim_start());
        match token {
            // `/report` reads the capture only, so the modes have no meaning
            // there: they stay unknown flags under that spelling.
            "--snapshot" | "--refresh" if !report => {
                let next = if token == "--snapshot" {
                    ExportMode::Snapshot
                } else {
                    ExportMode::Refresh
                };
                if let Some(previous) = mode
                    && previous != next
                {
                    return Err(SlashParseError(format!(
                        "use either --snapshot or --refresh, not both{usage}"
                    )));
                }
                mode = Some(next);
            }
            "--overwrite" => overwrite = true,
            "--rows" => {
                let (value, after) = take_value(remainder);
                if value.is_empty() {
                    return Err(SlashParseError(format!("--rows needs a number{usage}")));
                }
                let parsed = value
                    .parse::<usize>()
                    .map_err(|_| SlashParseError(format!("--rows needs a number{usage}")))?;
                if parsed > MAX_REPORT_ROWS {
                    return Err(SlashParseError(format!(
                        "--rows is capped at {MAX_REPORT_ROWS}{usage}"
                    )));
                }
                if rows.is_some() {
                    return Err(SlashParseError(format!("--rows given twice{usage}")));
                }
                rows = Some(parsed);
                rest = after;
                continue;
            }
            "--format" => {
                let (value, after) = take_value(remainder);
                if value.is_empty() {
                    return Err(SlashParseError(format!(
                        "--format needs a value (csv, json or md){usage}"
                    )));
                }
                let Some(parsed) = ExportFormat::parse(value) else {
                    return Err(SlashParseError(format!(
                        "--format csv|json|md, got: {value}{usage}"
                    )));
                };
                if report && parsed != ExportFormat::Markdown {
                    return Err(SlashParseError(format!(
                        "a report is Markdown: use /export --format {} for a data file{usage}",
                        parsed.word()
                    )));
                }
                if format.is_some() {
                    return Err(SlashParseError(format!("--format given twice{usage}")));
                }
                format = Some(parsed);
                rest = after;
                continue;
            }
            other if other.starts_with("--") => {
                return Err(SlashParseError(format!(
                    "unknown {spelling} flag: {other}{usage}"
                )));
            }
            _ => break,
        }
        rest = remainder;
    }
    let path = rest.trim();
    if path.is_empty() {
        return Err(SlashParseError(format!(
            "{spelling} requires a file path, e.g. /{spelling} out.{}",
            if report { "md" } else { "csv" }
        )));
    }
    if report
        && matches!(
            ExportFormat::of_path(path),
            Some(ExportFormat::Csv | ExportFormat::Json)
        )
    {
        return Err(SlashParseError(format!(
            "a report is Markdown text; {path} looks like a data file — use /export for \
             data files{usage}"
        )));
    }
    Ok(ExportRequest {
        mode,
        overwrite,
        rows,
        format: if report {
            Some(ExportFormat::Markdown)
        } else {
            format
        },
        path: path.to_string(),
        via_report_alias: report,
    })
}

/// The next whitespace-delimited token of `tail`, and the rest after it — the
/// one splitter a flag value uses, so `--rows`/`--format` consume exactly one
/// word and the path keeps everything else verbatim.
fn take_value(tail: &str) -> (&str, &str) {
    let end = tail.find(char::is_whitespace).unwrap_or(tail.len());
    (&tail[..end], tail[end..].trim_start())
}
