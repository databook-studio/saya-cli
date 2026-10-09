/// Parses the `/preferences` tail: bare or `list` lists; `add <text>` and
/// `edit <id> <text>` keep the text verbatim (outer ends trimmed, interior
/// spacing intact); `forget <id>` takes exactly one id; `forget-all` takes
/// nothing. Every malformed form is a usage error naming its remedy.
fn parse_preferences(tail: &str) -> Result<PreferencesCommand, SlashParseError> {
    const USAGE: &str = " (usage: /preferences, /preferences add <text>, \
         /preferences edit <id> <text>, /preferences forget <id>, /preferences forget-all)";
    if tail.is_empty() {
        return Ok(PreferencesCommand::List);
    }
    let (head, rest) = tail.split_once(char::is_whitespace).unwrap_or((tail, ""));
    let rest = rest.trim();
    match head {
        "list" => {
            if rest.is_empty() {
                Ok(PreferencesCommand::List)
            } else {
                Err(SlashParseError(format!(
                    "preferences list takes no argument{USAGE}"
                )))
            }
        }
        "add" => {
            if rest.is_empty() {
                return Err(SlashParseError(
                    "preferences add requires text: /preferences add <text>".into(),
                ));
            }
            Ok(PreferencesCommand::Add {
                text: rest.to_string(),
            })
        }
        "edit" => {
            let (id, text) = rest.split_once(char::is_whitespace).ok_or_else(|| {
                SlashParseError(
                    "preferences edit requires an id and text: /preferences edit <id> <text>"
                        .into(),
                )
            })?;
            let (id, text) = (id.trim(), text.trim());
            if id.is_empty() || text.is_empty() {
                return Err(SlashParseError(
                    "preferences edit requires an id and text: /preferences edit <id> <text>"
                        .into(),
                ));
            }
            Ok(PreferencesCommand::Edit {
                id: id.to_string(),
                text: text.to_string(),
            })
        }
        "forget" => {
            if rest.is_empty() {
                return Err(SlashParseError(
                    "preferences forget requires an id: /preferences forget <id>".into(),
                ));
            }
            if rest.split_whitespace().count() > 1 {
                return Err(SlashParseError(
                    "preferences forget takes exactly one id: /preferences forget <id>".into(),
                ));
            }
            Ok(PreferencesCommand::Forget {
                id: rest.to_string(),
            })
        }
        "forget-all" => {
            if !rest.is_empty() {
                return Err(SlashParseError(format!(
                    "preferences forget-all takes no argument{USAGE}"
                )));
            }
            Ok(PreferencesCommand::ForgetAll)
        }
        other => Err(SlashParseError(format!(
            "unknown /preferences subcommand: {other}{USAGE}"
        ))),
    }
}
