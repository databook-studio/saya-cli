use crate::cli::ContractsCommand;
use saya_agent::{AgentMode, ApprovalPolicy};
use std::{fmt, str::FromStr};

mod contracts;
mod help;
pub(crate) mod registry;

// Re-exported so the session command layer's `crate::slash::help_for` path
// still resolves after the help text moved to `help.rs`.
pub(crate) use help::help_for;
// The bypass-composition sentence is shared by the `/mode` help entry and the
// `/mode` answers, so it is re-exported for the session command layer.
pub(crate) use help::PLAN_BYPASS_SENTENCE;
// the one-line description per command is the single source shared by the
// `/help` listing and the completion popup (`interactive::tui::complete`), so
// the two surfaces cannot drift. `description_for` is read by the popup in
// production; `COMMAND_DESCRIPTIONS` is only needed by tests that assert the
// shared table covers the registry, so it is re-exported under `cfg(test)`.
#[cfg(test)]
pub(crate) use help::COMMAND_DESCRIPTIONS;
pub(crate) use help::description_for;
// The run scope parser's parity test reads the `/run` help from
// `commands/run/scopes.rs`, so the per-command help joins the re-exports.
#[cfg(test)]
pub(crate) use help::command_help;
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
    Mode(Option<AgentMode>),
    Schema(bool),
    Sql(String),
    Export(String),
    Chart(String),
    Explain(String),
    Clear,
    History,
    Sessions,
    Resume(String),
    /// Choose which columns wide result tables show in the TUI:
    /// `/columns name1,name2` filters; `/columns` or `/columns all` resets.
    Columns(Option<String>),
    /// A contract slash command (`/contracts`, `/contract`, `/remember`,
    /// `/forget`), already translated to the same `ContractsCommand` the
    /// headless `saya contracts` parser produces. The adapter slice (2b-4)
    /// hands it to the shared `run_contracts` dispatcher — no second parsing.
    Contracts(ContractsCommand),
    /// `/run <tail>` — start or operate a headless run from the session. The
    /// raw tail is handed to a nested `saya run` child process verbatim, whose
    /// output passes through the parent's stdout/stderr unmangled (the
    /// dual-tag hazard is documented where the child spawns,
    /// `interactive::session_run`). The child's own CLI parser stays the
    /// authority on `--allow`, `--budget`, and the `cancel`/`resume`/`show`
    /// subcommands — the adapter parses nothing twice, except its one word:
    /// a leading `--seed-grants` requests that the child's `--allow` be
    /// seeded from this session's grants (accepted tokens forwarded, refused
    /// ones named), and is stripped before the child parses.
    Run(String),
    /// `/run cancel <id>` — record a run cancelled through the same engine
    /// path `saya run cancel` uses, via the shared dispatcher.
    RunCancel(String),
    /// `/runs [id]` — list every run, or show one run when you name its id,
    /// through the same read path the headless `saya run list|show` commands
    /// use.
    Runs(Option<String>),
    /// `/allow <scopes…>` — seed pre-authorisation into the session's grant
    /// store: the same `--allow` grammar, judged on the session surface.
    /// The tokens are carried as stated; the shared `session_grants`
    /// behaviour does the parsing, the seeding, and the message (`none`
    /// keeps its grammar meaning: the empty approval, alone — it seeds
    /// nothing and is not a revoke).
    Allow(Vec<String>),
    /// `/grants` — the session's grant store listed verbatim: one token per
    /// line, sorted, under a header stating the lifetime.
    Grants,
    /// Run `config doctor` in-session: secrets resolve? provider endpoint?
    Doctor,
    /// Show session token usage totals and cache hit rate.
    Usage,
    /// Toggle display of the model's chain-of-thought in the transcript.
    Thinking(Option<bool>),
    /// Show the session's workspace binding: the pinned canonical root, or
    /// the no-root shape where nothing binds.
    Workspace,
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
        "mode" => SlashCommand::Mode(parse_mode(&arg)?),
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
        "columns" => SlashCommand::Columns((!arg.is_empty()).then_some(arg)),
        "doctor" => SlashCommand::Doctor,
        "usage" => SlashCommand::Usage,
        "thinking" => SlashCommand::Thinking(parse_bool(&arg)?),
        "workspace" => SlashCommand::Workspace,
        "contracts" | "contract" | "remember" | "forget" | "queue" | "confirm" | "reject"
        | "approve-all" => {
            // The contract slash adapters: translate to the same
            // `ContractsCommand` the headless parser produces and hand it to
            // the shared dispatcher. No second parsing or DTO mapping lives here.
            // `confirm`/`reject` translate to `ContractsCommand::Decide`.
            return contracts::parse_contract_command(name, &arg)
                .map(|maybe| maybe.map(SlashCommand::Contracts));
        }
        "run" => {
            // `/run` and `/run cancel <id>`. The subcommand word is matched
            // first, exactly as the headless `saya run` CLI disambiguates:
            // `saya run cancel <id>` is the Cancel subcommand, so a slash
            // goal beginning with "cancel" needs the same treatment the
            // headless grammar already gives it. Everything else is the
            // nested-run tail, handed to the child verbatim.
            let tail = arg.trim();
            if tail.is_empty() {
                return Err(SlashParseError(
                    "run requires a goal and --allow scopes, e.g. \
                     /run survey the data --allow workspace-write"
                        .into(),
                ));
            }
            let (head, rest) = tail.split_once(' ').unwrap_or((tail, ""));
            if head == "cancel" {
                let run_id = rest.trim();
                if run_id.is_empty() {
                    return Err(SlashParseError(
                        "run cancel requires a run id: /run cancel <id>".into(),
                    ));
                }
                SlashCommand::RunCancel(run_id.to_string())
            } else {
                SlashCommand::Run(tail.to_string())
            }
        }
        "runs" => SlashCommand::Runs((!arg.is_empty()).then_some(arg)),
        "allow" => {
            // `/allow <scopes…>` seeds the session's grant store; `/allow`
            // alone is not a listing — `/grants` is the listing, and the
            // error says where to find it.
            if arg.is_empty() {
                return Err(SlashParseError(
                    "allow requires scopes to seed, e.g. /allow sql:analytics \
                     (or `/allow none` for the empty approval); /allow alone is \
                     not a listing — use /grants to list"
                        .into(),
                ));
            }
            SlashCommand::Allow(arg.split_whitespace().map(str::to_string).collect())
        }
        "grants" => SlashCommand::Grants,
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
        _ => Err(SlashParseError("expected on or off".into())),
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

fn parse_mode(value: &str) -> Result<Option<AgentMode>, SlashParseError> {
    if value.is_empty() {
        return Ok(None);
    }
    AgentMode::from_str(value).map(Some).map_err(|_| {
        SlashParseError(format!(
            "invalid agent mode: {value} (expected plan or build)"
        ))
    })
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
    fn test_parse_columns_command() {
        assert_eq!(
            parse_slash_command("/columns"),
            Ok(Some(SlashCommand::Columns(None)))
        );
        assert_eq!(
            parse_slash_command("/columns id, total"),
            Ok(Some(SlashCommand::Columns(Some("id, total".into()))))
        );
        assert_eq!(
            parse_slash_command("/columns all"),
            Ok(Some(SlashCommand::Columns(Some("all".into()))))
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

    /// `/history` and `/sessions` are both still known commands, and
    /// `exit`/`quit` are a
    /// deliberate conventional alias pair. The typo suggester
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

    /// `/run` keeps its tail verbatim — the nested child's parser is the
    /// authority on it — and `/run cancel <id>` is the subcommand form, the
    /// same disambiguation the headless `saya run` grammar gives.
    #[test]
    fn test_parse_run_and_run_cancel() {
        assert_eq!(
            parse_slash_command("/run survey the data --allow workspace-write"),
            Ok(Some(SlashCommand::Run(
                "survey the data --allow workspace-write".into()
            )))
        );
        assert_eq!(
            parse_slash_command("/run resume r-1"),
            Ok(Some(SlashCommand::Run("resume r-1".into())))
        );
        assert_eq!(
            parse_slash_command("/run cancel r-1"),
            Ok(Some(SlashCommand::RunCancel("r-1".into())))
        );
        assert_eq!(
            parse_slash_command("/run  cancel  r-1 "),
            Ok(Some(SlashCommand::RunCancel("r-1".into())))
        );
        // A bare /run and a cancelless id are usage errors with guidance.
        assert!(parse_slash_command("/run").is_err());
        assert!(parse_slash_command("/run cancel").is_err());
    }

    /// `/runs` lists without an id and shows one with it.
    #[test]
    fn test_parse_runs() {
        assert_eq!(
            parse_slash_command("/runs"),
            Ok(Some(SlashCommand::Runs(None)))
        );
        assert_eq!(
            parse_slash_command("/runs r-1"),
            Ok(Some(SlashCommand::Runs(Some("r-1".into()))))
        );
    }

    /// `/allow` carries the scopes as stated, whitespace-separated, and
    /// `/allow` alone is not a listing — it is a usage error pointing at
    /// the listing (`/grants`), which parses bare.
    #[test]
    fn test_parse_allow_and_grants() {
        assert_eq!(
            parse_slash_command("/allow sql:analytics"),
            Ok(Some(SlashCommand::Allow(vec!["sql:analytics".into()])))
        );
        assert_eq!(
            parse_slash_command("/allow sql:analytics runner:bench"),
            Ok(Some(SlashCommand::Allow(vec![
                "sql:analytics".into(),
                "runner:bench".into()
            ])))
        );
        assert_eq!(
            parse_slash_command("/grants"),
            Ok(Some(SlashCommand::Grants))
        );
        // Trailing arguments are ignored, like /doctor's.
        assert_eq!(
            parse_slash_command("/grants now"),
            Ok(Some(SlashCommand::Grants))
        );
        let error = parse_slash_command("/allow").unwrap_err();
        assert!(
            error.0.contains("not a listing") && error.0.contains("/grants"),
            "the error points at the listing, got: {error}"
        );
    }

    /// `/mode` reports bare, switches on `plan`/`build` through the
    /// `AgentMode::FromStr` grammar, and refuses anything else naming both
    /// valid values. Matching `/approvals`'s case handling: the parse is
    /// case-sensitive, so `Plan` is an error, not a mode.
    #[test]
    fn test_parse_mode_reports_switches_and_refuses() {
        use saya_agent::AgentMode;
        assert_eq!(
            parse_slash_command("/mode"),
            Ok(Some(SlashCommand::Mode(None)))
        );
        assert_eq!(
            parse_slash_command("/mode plan"),
            Ok(Some(SlashCommand::Mode(Some(AgentMode::Plan))))
        );
        assert_eq!(
            parse_slash_command("/mode build"),
            Ok(Some(SlashCommand::Mode(Some(AgentMode::Build))))
        );
        let error = parse_slash_command("/mode nonsense").unwrap_err();
        assert!(
            error.0.contains("plan") && error.0.contains("build"),
            "the error names both valid values, got: {error}"
        );
        assert!(
            parse_slash_command("/mode Plan").is_err(),
            "the parse is case-sensitive like /approvals"
        );
    }

    /// `/mode` is not bypass consent: it must not trip the journalling gate
    /// the two executors consult (`session_loop.rs` and `tui/dispatch.rs`
    /// match on `SlashCommand::Approvals(Some(_))`). A mode change — report
    /// or switch — never matches that shape.
    #[test]
    fn mode_never_matches_the_bypass_consent_gate() {
        for command in [
            parse_slash_command("/mode").unwrap().unwrap(),
            parse_slash_command("/mode plan").unwrap().unwrap(),
            parse_slash_command("/mode build").unwrap().unwrap(),
        ] {
            assert!(
                !matches!(command, SlashCommand::Approvals(Some(_))),
                "a mode change is not bypass consent: {command:?}"
            );
        }
        assert!(
            matches!(
                parse_slash_command("/approvals bypass").unwrap().unwrap(),
                SlashCommand::Approvals(Some(_))
            ),
            "the gate still matches its own /approvals shape"
        );
    }
}
