/// Parses the `/recovery` tail: bare or `status` shows the status; `on`
/// and `off` flip the session flag; `list` lists the recorded edits;
/// `diff <id>` and `restore <id>` take exactly one id. No free text:
/// every malformed form is a usage error naming its remedy.
fn parse_recovery(tail: &str) -> Result<RecoveryCommand, SlashParseError> {
    const USAGE: &str = " (usage: /recovery, /recovery on, /recovery off, /recovery list, \
         /recovery diff <id>, /recovery restore <id>)";
    if tail.is_empty() {
        return Ok(RecoveryCommand::Status);
    }
    let (head, rest) = tail.split_once(char::is_whitespace).unwrap_or((tail, ""));
    let rest = rest.trim();
    match head {
        "status" if rest.is_empty() => Ok(RecoveryCommand::Status),
        "on" if rest.is_empty() => Ok(RecoveryCommand::On),
        "off" if rest.is_empty() => Ok(RecoveryCommand::Off),
        "list" if rest.is_empty() => Ok(RecoveryCommand::List),
        "status" | "on" | "off" | "list" => Err(SlashParseError(format!(
            "recovery {head} takes no argument{USAGE}"
        ))),
        "diff" | "restore" => {
            if rest.is_empty() {
                return Err(SlashParseError(format!(
                    "recovery {head} requires an id: /recovery {head} <id>"
                )));
            }
            if rest.split_whitespace().count() > 1 {
                return Err(SlashParseError(format!(
                    "recovery {head} takes exactly one id"
                )));
            }
            Ok(if head == "diff" {
                RecoveryCommand::Diff(rest.to_string())
            } else {
                RecoveryCommand::Restore(rest.to_string())
            })
        }
        other => Err(SlashParseError(format!(
            "unknown /recovery subcommand: {other} (usage: /recovery, /recovery on, \
             /recovery off, /recovery list, /recovery diff <id>, /recovery restore <id>)"
        ))),
    }
}
