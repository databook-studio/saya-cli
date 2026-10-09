/// Parses the `/export` tail: leading flags in any order, then the path —
/// the rest of the tail verbatim. Unknown `--flag`s, both modes together,
/// and a missing path are usage errors, so a flag is never read as path
/// text and a path is never read as a flag.
fn parse_export(tail: &str) -> Result<ExportRequest, SlashParseError> {
    const USAGE: &str = " (usage: /export [--snapshot|--refresh] [--overwrite] <path>)";
    let mut mode: Option<ExportMode> = None;
    let mut overwrite = false;
    let mut rest = tail;
    loop {
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let (token, remainder) = (&rest[..end], rest[end..].trim_start());
        match token {
            "--snapshot" | "--refresh" => {
                let next = if token == "--snapshot" {
                    ExportMode::Snapshot
                } else {
                    ExportMode::Refresh
                };
                if let Some(previous) = mode
                    && previous != next
                {
                    return Err(SlashParseError(format!(
                        "use either --snapshot or --refresh, not both{USAGE}"
                    )));
                }
                mode = Some(next);
            }
            "--overwrite" => overwrite = true,
            other if other.starts_with("--") => {
                return Err(SlashParseError(format!(
                    "unknown export flag: {other}{USAGE}"
                )));
            }
            _ => break,
        }
        rest = remainder;
    }
    let path = rest.trim();
    if path.is_empty() {
        return Err(SlashParseError(
            "export requires a file path, e.g. /export out.csv".into(),
        ));
    }
    Ok(ExportRequest {
        mode,
        overwrite,
        path: path.to_string(),
    })
}
