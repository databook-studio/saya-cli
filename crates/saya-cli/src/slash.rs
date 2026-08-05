use saya_agent::ApprovalPolicy;
use std::{fmt, str::FromStr};

/// Known slash command names handled by `parse_slash_command`.
const KNOWN_COMMANDS: &[&str] = &[
    "connect",
    "connections",
    "include",
    "exclude",
    "provider",
    "model",
    "privacy",
    "approvals",
    "schema",
    "sql",
    "clear",
    "history",
    "sessions",
    "resume",
    "help",
    "exit",
    "quit",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlashCommand {
    Connect(String),
    Connections,
    Include(String),
    Exclude(String),
    Provider(Option<String>),
    Model(Option<String>),
    Privacy(Option<bool>),
    Approvals(Option<ApprovalPolicy>),
    Schema(bool),
    Sql(String),
    Clear,
    History,
    Sessions,
    Resume(String),
    Help(Option<String>),
    Exit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlashParseError(pub String);

impl fmt::Display for SlashParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for SlashParseError {}

pub fn parse_slash_command(input: &str) -> Result<Option<SlashCommand>, SlashParseError> {
    let trimmed = input.trim();
    if !trimmed.starts_with('/') {
        return Ok(None);
    }
    let mut parts = trimmed[1..].split_whitespace();
    let name = parts.next().unwrap_or_default();
    let arg = parts.collect::<Vec<_>>().join(" ");
    let required = || {
        (!arg.is_empty())
            .then_some(arg.clone())
            .ok_or_else(|| SlashParseError("command requires an argument".into()))
    };
    let command = match name {
        "connect" => SlashCommand::Connect(required()?),
        "connections" => SlashCommand::Connections,
        "include" => SlashCommand::Include(required()?),
        "exclude" => SlashCommand::Exclude(required()?),
        "provider" => SlashCommand::Provider((!arg.is_empty()).then_some(arg)),
        "model" => SlashCommand::Model((!arg.is_empty()).then_some(arg)),
        "privacy" => SlashCommand::Privacy(parse_bool(&arg)?),
        "approvals" => SlashCommand::Approvals(parse_approval(&arg)?),
        "schema" => SlashCommand::Schema(arg == "refresh"),
        "sql" => {
            let query = trimmed.strip_prefix("/sql").unwrap_or("").trim();
            if query.is_empty() {
                return Err(SlashParseError("sql requires a query".into()));
            }
            SlashCommand::Sql(query.to_string())
        }
        "clear" => SlashCommand::Clear,
        "history" => SlashCommand::History,
        "sessions" => SlashCommand::Sessions,
        "resume" => SlashCommand::Resume(required()?),
        "help" => SlashCommand::Help((!arg.is_empty()).then_some(arg)),
        "exit" | "quit" => SlashCommand::Exit,
        other => {
            let msg = match closest_command(other) {
                Some(sugg) => format!("unknown command: /{other} (did you mean /{sugg}?)"),
                None => format!("unknown command: /{other}"),
            };
            return Err(SlashParseError(msg));
        }
    };
    Ok(Some(command))
}

/// Calculates the Levenshtein edit distance between two strings using a single rolling row.
fn levenshtein(a: &str, b: &str) -> usize {
    let b_chars: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b_chars.len()).collect();

    for (i, ca) in a.chars().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for (j, &cb) in b_chars.iter().enumerate() {
            let old_row_j_plus_1 = row[j + 1];
            let cost = if ca == cb { 0 } else { 1 };
            row[j + 1] = (prev + cost).min(row[j] + 1).min(old_row_j_plus_1 + 1);
            prev = old_row_j_plus_1;
        }
    }

    row.last().copied().unwrap_or(0)
}

/// Returns the known command with the smallest Levenshtein distance to `input` if distance <= 2.
fn closest_command(input: &str) -> Option<&'static str> {
    let input_lower = input.to_lowercase();
    let mut best_cmd = None;
    let mut min_dist = usize::MAX;

    for &cmd in KNOWN_COMMANDS {
        let dist = levenshtein(&input_lower, cmd);
        if dist < min_dist {
            min_dist = dist;
            best_cmd = Some(cmd);
        }
    }

    if min_dist <= 2 { best_cmd } else { None }
}

fn parse_bool(value: &str) -> Result<Option<bool>, SlashParseError> {
    if value.is_empty() {
        return Ok(None);
    }
    match value {
        "on" | "true" | "enable" => Ok(Some(true)),
        "off" | "false" | "disable" => Ok(Some(false)),
        _ => Err(SlashParseError("privacy expects on or off".into())),
    }
}

fn parse_approval(value: &str) -> Result<Option<ApprovalPolicy>, SlashParseError> {
    if value.is_empty() {
        return Ok(None);
    }
    ApprovalPolicy::from_str(value)
        .map(Some)
        .map_err(|error| SlashParseError(error.to_string()))
}

pub fn help_text() -> &'static str {
    "/connect <profile>  /connections  /include <profile>  /exclude <profile>\n/provider [name]     /model [name]  /privacy [on|off]\n/approvals [ask|read-only|never]  /schema [refresh]  /sql <query>  /clear\n/history  /sessions  /resume <id>  /help  /exit"
}

/// Returns a short usage and example string for a known slash command, or `None` if unknown.
pub fn command_help(name: &str) -> Option<&'static str> {
    let clean_name = name.trim_start_matches('/').to_lowercase();
    match clean_name.as_str() {
        "connect" => {
            Some("connect <profile> — set the active database profile. Example: /connect prod")
        }
        "connections" => Some(
            "connections — list configured database connection profiles. Example: /connections",
        ),
        "include" => Some(
            "include <profile> — include an additional database profile. Example: /include staging",
        ),
        "exclude" => {
            Some("exclude <profile> — exclude a database profile. Example: /exclude staging")
        }
        "provider" => {
            Some("provider [name] — view or set the AI provider. Example: /provider anthropic")
        }
        "model" => Some("model [name] — view or set the AI model. Example: /model gpt-4o"),
        "privacy" => {
            Some("privacy [on|off] — view or toggle cloud data sharing. Example: /privacy off")
        }
        "approvals" => Some(
            "approvals [ask|read-only|never] — view or set tool execution approval policy. Example: /approvals ask",
        ),
        "schema" => Some(
            "schema [refresh] — display or refresh database schema context. Example: /schema refresh",
        ),
        "sql" => Some(
            "sql <query> — execute a raw SQL query directly. Example: /sql SELECT * FROM users LIMIT 10;",
        ),
        "clear" => Some("clear — clear conversation history and context. Example: /clear"),
        "history" => Some("history — display session history. Example: /history"),
        "sessions" => Some("sessions — list available interactive sessions. Example: /sessions"),
        "resume" => Some("resume <id> — resume a previous session by ID. Example: /resume 12345"),
        "help" => Some(
            "help [command] — display general help or detailed usage for a command. Example: /help connect",
        ),
        "exit" | "quit" => Some("exit — exit the interactive CLI session. Example: /exit"),
        _ => None,
    }
}

/// Returns command-specific help for a topic, or general help text if `topic` is `None`.
pub fn help_for(topic: Option<&str>) -> String {
    match topic {
        Some(name) => {
            let clean = name.trim_start_matches('/');
            match command_help(clean) {
                Some(help) => help.to_string(),
                None => format!("No help for /{clean}. Type /help for the full list."),
            }
        }
        None => help_text().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_help_command() {
        assert_eq!(
            parse_slash_command("/help"),
            Ok(Some(SlashCommand::Help(None)))
        );
        assert_eq!(
            parse_slash_command("/help connect"),
            Ok(Some(SlashCommand::Help(Some("connect".into()))))
        );

        let help_connect = help_for(Some("connect"));
        assert!(help_connect.contains("connect"));
        assert!(help_connect.contains("Example"));

        let help_unknown = help_for(Some("nope"));
        assert!(help_unknown.contains("No help"));

        assert_eq!(help_for(None), help_text().to_string());
    }

    #[test]
    fn test_parse_sessions_and_resume() {
        assert_eq!(
            parse_slash_command("/sessions"),
            Ok(Some(SlashCommand::Sessions))
        );
        assert_eq!(
            parse_slash_command("/resume 12345"),
            Ok(Some(SlashCommand::Resume("12345".into())))
        );
        assert_eq!(
            parse_slash_command("/resume"),
            Err(SlashParseError("command requires an argument".into()))
        );
    }

    #[test]
    fn test_parse_sql_command() {
        assert_eq!(
            parse_slash_command("/sql SELECT * FROM users;"),
            Ok(Some(SlashCommand::Sql("SELECT * FROM users;".into())))
        );
        assert_eq!(
            parse_slash_command("/sql   SELECT  a,  b  FROM  table  "),
            Ok(Some(SlashCommand::Sql("SELECT  a,  b  FROM  table".into())))
        );
        assert_eq!(
            parse_slash_command("/sql"),
            Err(SlashParseError("sql requires a query".into()))
        );
        assert_eq!(
            parse_slash_command("/sql   "),
            Err(SlashParseError("sql requires a query".into()))
        );
    }

    #[test]
    fn test_unknown_command_suggestion() {
        let err = parse_slash_command("/conect prod").unwrap_err();
        assert!(
            err.0.contains("did you mean /connect"),
            "expected suggestion in error message, got: {}",
            err.0
        );

        let err = parse_slash_command("/zzzzzzzz").unwrap_err();
        assert!(
            !err.0.contains("did you mean"),
            "unexpected suggestion in error message, got: {}",
            err.0
        );

        assert_eq!(
            parse_slash_command("/connect prod"),
            Ok(Some(SlashCommand::Connect("prod".into())))
        );
    }
}
