use crate::cli::ContractsCommand;
use saya_agent::ApprovalPolicy;
use std::{fmt, str::FromStr};

mod contracts;
mod help;
pub(crate) mod registry;

// Re-exported so the session command layer's `crate::slash::help_for` path
// still resolves after the help text moved to `help.rs`.
pub(crate) use help::help_for;
// The inline `test_help_command` test calls `help_text` bare via `super::*`;
// bring it into scope for tests only so the test stays unchanged.
#[cfg(test)]
use help::help_text;

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
    Export(String),
    Chart(String),
    Explain(String),
    Clear,
    History,
    Sessions,
    Resume(String),
    /// A contract slash command (`/contracts`, `/contract`, `/remember`,
    /// `/forget`), already translated to the same `ContractsCommand` the
    /// headless `saya contracts` parser produces. The adapter slice (2b-4)
    /// hands it to the shared `run_contracts` dispatcher — no second parsing.
    Contracts(ContractsCommand),
    /// Run `config doctor` in-session: secrets resolve? provider endpoint?
    Doctor,
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
        "export" => {
            let path = trimmed.strip_prefix("/export").unwrap_or("").trim();
            if path.is_empty() {
                return Err(SlashParseError(
                    "export requires a file path, e.g. /export out.csv".into(),
                ));
            }
            SlashCommand::Export(path.to_string())
        }
        "chart" => SlashCommand::Chart(arg.trim().to_string()),
        "explain" => SlashCommand::Explain(arg.trim().to_string()),
        "clear" => SlashCommand::Clear,
        "history" => SlashCommand::History,
        "sessions" => SlashCommand::Sessions,
        "resume" => SlashCommand::Resume(required()?),
        "doctor" => SlashCommand::Doctor,
        "contracts" | "contract" | "remember" | "forget" | "queue" | "confirm" | "reject" => {
            // The contract slash adapters: translate to the same
            // `ContractsCommand` the headless parser produces and hand it to the
            // shared dispatcher. No second parsing or DTO mapping lives here.
            // `confirm`/`reject` (spec D) translate to `ContractsCommand::Decide`.
            return contracts::parse_contract_command(name, &arg)
                .map(|maybe| maybe.map(SlashCommand::Contracts));
        }
        "help" => SlashCommand::Help((!arg.is_empty()).then_some(arg)),
        "exit" | "quit" => SlashCommand::Exit,
        other => {
            let msg = match registry::closest_command(other) {
                Some(sugg) => format!("unknown command: /{other} (did you mean /{sugg}?)"),
                None => format!("unknown command: /{other}"),
            };
            return Err(SlashParseError(msg));
        }
    };
    Ok(Some(command))
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
    fn test_parse_doctor_takes_no_arguments() {
        assert!(matches!(
            parse_slash_command("/doctor").unwrap(),
            Some(SlashCommand::Doctor)
        ));
        // Like /clear and /history, a trailing argument is ignored.
        assert!(matches!(
            parse_slash_command("/doctor now").unwrap(),
            Some(SlashCommand::Doctor)
        ));
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
    fn test_parse_export_command() {
        assert_eq!(
            parse_slash_command("/export out.csv"),
            Ok(Some(SlashCommand::Export("out.csv".into())))
        );
        assert_eq!(
            parse_slash_command("/export"),
            Err(SlashParseError(
                "export requires a file path, e.g. /export out.csv".into()
            ))
        );
    }

    #[test]
    fn test_parse_chart_command() {
        assert_eq!(
            parse_slash_command("/chart"),
            Ok(Some(SlashCommand::Chart("".into())))
        );
        assert_eq!(
            parse_slash_command("/chart foo"),
            Ok(Some(SlashCommand::Chart("foo".into())))
        );
    }

    #[test]
    fn test_parse_explain_command() {
        assert_eq!(
            parse_slash_command("/explain"),
            Ok(Some(SlashCommand::Explain("".into())))
        );
        assert_eq!(
            parse_slash_command("/explain SELECT 1"),
            Ok(Some(SlashCommand::Explain("SELECT 1".into())))
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

    /// `/history` and `/sessions` are both still known commands (S11 keeps
    /// `/history` as an explicit alias of `/sessions`), and `exit`/`quit` are a
    /// deliberate conventional alias pair. Invariant 4: the typo suggester
    /// must still resolve anything it resolved before for names that still
    /// exist — so a near-miss on each lands on the kept name, never on a
    /// removed one.
    #[test]
    fn kept_alias_pairs_still_parse_and_suggest() {
        // Both names still parse.
        assert_eq!(
            parse_slash_command("/history"),
            Ok(Some(SlashCommand::History))
        );
        assert_eq!(
            parse_slash_command("/sessions"),
            Ok(Some(SlashCommand::Sessions))
        );
        assert_eq!(parse_slash_command("/exit"), Ok(Some(SlashCommand::Exit)));
        assert_eq!(parse_slash_command("/quit"), Ok(Some(SlashCommand::Exit)));

        // A one-char typo on a kept name suggests that name, not something else.
        assert_eq!(registry::closest_command("histor"), Some("history"));
        assert_eq!(registry::closest_command("session"), Some("sessions"));
        // `quit` is a deliberate alias of `exit`; a near-miss still lands on a
        // known name (the suggester picks the closest, never a removed one).
        assert_eq!(registry::closest_command("exi"), Some("exit"));
    }
}
