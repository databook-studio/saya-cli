/// Parses the `/undo` tail: the command takes no argument, so a trailing word
/// is a mistake worth naming rather than a silent ignore.
fn parse_undo(tail: &str) -> Result<(), SlashParseError> {
    if !tail.trim().is_empty() {
        return Err(SlashParseError(
            "undo takes no argument: /undo undoes the last turn's file edits, and /rewind <id> \
             restores a named checkpoint"
                .into(),
        ));
    }
    Ok(())
}

/// Parses the `/rewind` tail: bare lists the run's checkpoints, one word names
/// the checkpoint to restore. Two words are a usage error — a rewind states one
/// checkpoint — and the id is otherwise carried as stated.
fn parse_rewind(tail: &str) -> Result<RewindRequest, SlashParseError> {
    let id = tail.trim();
    if id.is_empty() {
        return Ok(RewindRequest::List);
    }
    if id.split_whitespace().count() > 1 {
        return Err(SlashParseError(
            "rewind takes one checkpoint id: /rewind [<id>]".into(),
        ));
    }
    Ok(RewindRequest::Restore(id.to_string()))
}
