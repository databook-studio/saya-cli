/// Parses the `/notes` tail: bare or `show` shows; `set <text>` keeps the
/// text verbatim (outer ends trimmed, interior spacing intact); `clear`
/// takes nothing. Every malformed form is a usage error naming its remedy.
fn parse_notes(tail: &str) -> Result<NotesCommand, SlashParseError> {
    const USAGE: &str = " (usage: /notes, /notes set <text>, /notes clear)";
    if tail.is_empty() {
        return Ok(NotesCommand::Show);
    }
    let (head, rest) = tail.split_once(char::is_whitespace).unwrap_or((tail, ""));
    let rest = rest.trim();
    match head {
        "show" => {
            if rest.is_empty() {
                Ok(NotesCommand::Show)
            } else {
                Err(SlashParseError(format!(
                    "notes show takes no argument{USAGE}"
                )))
            }
        }
        "set" => {
            if rest.is_empty() {
                return Err(SlashParseError(
                    "notes set requires text: /notes set <text>".into(),
                ));
            }
            Ok(NotesCommand::Set(rest.to_string()))
        }
        "clear" => {
            if !rest.is_empty() {
                return Err(SlashParseError(format!(
                    "notes clear takes no argument{USAGE}"
                )));
            }
            Ok(NotesCommand::Clear)
        }
        other => Err(SlashParseError(format!(
            "unknown /notes subcommand: {other} (usage: /notes, /notes set <text>, /notes clear)"
        ))),
    }
}
