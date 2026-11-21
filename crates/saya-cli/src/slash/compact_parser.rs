/// Parses the `/compact` tail: bare compacts this session's working memory;
/// `--new-session` compacts and continues in a smaller successor, which is the
/// one operation `/rotate` has always run — its one-release alias states the
/// same spelling. Anything else is a usage error naming both forms, so a typo
/// cannot read as a silent compaction.
fn parse_compact(tail: &str) -> Result<CompactRequest, SlashParseError> {
    match tail.trim() {
        "" => Ok(CompactRequest::compact()),
        "--new-session" => Ok(CompactRequest::new_session()),
        other => Err(SlashParseError(format!(
            "unknown /compact argument: {other} (usage: /compact, /compact --new-session)"
        ))),
    }
}
