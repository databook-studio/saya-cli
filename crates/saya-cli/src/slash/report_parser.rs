/// Parses the `/report` tail: leading `--rows N` / `--overwrite` flags in any
/// order, then the path — the rest of the tail verbatim. `--rows` takes the
/// next whitespace-delimited token as its value; a value above
/// [`MAX_REPORT_ROWS`], a non-number, a repeated `--rows`, and unknown
/// `--flag`s are usage errors, so a flag is never read as path text and a
/// path is never read as a flag. A `.csv`/`.json` destination is refused
/// with the hint that data files belong to `/export`.
fn parse_report(tail: &str) -> Result<ReportRequest, SlashParseError> {
    const USAGE: &str = " (usage: /report [--rows N] [--overwrite] <path>)";
    let mut rows: Option<usize> = None;
    let mut overwrite = false;
    let mut rest = tail;
    loop {
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let (token, remainder) = (&rest[..end], rest[end..].trim_start());
        match token {
            "--overwrite" => overwrite = true,
            "--rows" => {
                let value_end = remainder
                    .find(char::is_whitespace)
                    .unwrap_or(remainder.len());
                let (value, after) = (&remainder[..value_end], remainder[value_end..].trim_start());
                if value.is_empty() {
                    return Err(SlashParseError(format!("--rows needs a number{USAGE}")));
                }
                let parsed = value
                    .parse::<usize>()
                    .map_err(|_| SlashParseError(format!("--rows needs a number{USAGE}")))?;
                if parsed > MAX_REPORT_ROWS {
                    return Err(SlashParseError(format!(
                        "--rows is capped at {MAX_REPORT_ROWS}{USAGE}"
                    )));
                }
                if rows.is_some() {
                    return Err(SlashParseError(format!("--rows given twice{USAGE}")));
                }
                rows = Some(parsed);
                rest = after;
                continue;
            }
            other if other.starts_with("--") => {
                return Err(SlashParseError(format!(
                    "unknown report flag: {other}{USAGE}"
                )));
            }
            _ => break,
        }
        rest = remainder;
    }
    let path = rest.trim();
    if path.is_empty() {
        return Err(SlashParseError(
            "report requires a file path, e.g. /report out.md".into(),
        ));
    }
    let lowered = path.to_lowercase();
    if lowered.ends_with(".csv") || lowered.ends_with(".json") {
        return Err(SlashParseError(format!(
            "a report is Markdown text; {path} looks like a data file — use /export for \
             data files{USAGE}"
        )));
    }
    Ok(ReportRequest {
        path: path.to_string(),
        rows,
        overwrite,
    })
}
