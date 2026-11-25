/// The largest `/usage --limit` value, in US dollars: the same bound the
/// config's `[limits] session_cost_usd` states, so a typo cannot become an
/// effectively unbounded session.
const MAX_USAGE_LIMIT_USD: f64 = 1_000_000.0;

/// Parses the `/usage` tail: bare shows the panel, and `--limit <usd>` also
/// states this session's cost ceiling. Any other word is a usage error naming
/// both forms rather than a silent ignore.
fn parse_usage(tail: &str) -> Result<UsageRequest, SlashParseError> {
    let tail = tail.trim();
    if tail.is_empty() {
        return Ok(UsageRequest::show());
    }
    let words: Vec<&str> = tail.split_whitespace().collect();
    let ["--limit", value] = words.as_slice() else {
        return Err(SlashParseError(
            "usage takes no argument, or --limit <usd>: /usage [--limit <usd>]".into(),
        ));
    };
    let usd: f64 = value.parse().map_err(|_| {
        SlashParseError(format!(
            "usage --limit needs a positive number of US dollars, got {value}: \
             /usage --limit <usd>"
        ))
    })?;
    if !usd.is_finite() || usd <= 0.0 || usd > MAX_USAGE_LIMIT_USD {
        return Err(SlashParseError(format!(
            "usage --limit needs a positive number of US dollars up to {MAX_USAGE_LIMIT_USD:.0}, \
             got {value}: /usage --limit <usd>"
        )));
    }
    let micros = (usd * 1_000_000.0).round() as u64;
    if micros == 0 {
        return Err(SlashParseError(format!(
            "usage --limit is too small to measure, got {value}: /usage --limit <usd>"
        )));
    }
    Ok(UsageRequest::limit(micros))
}
