//! The session's job surface: `/job <verb>` for the durable-job lifecycle,
//! `/start` and `/follow-up` for the two agent launches, and one bare spelling
//! each for the six session-only controls.
//!
//! Every spelling parses through the one `CodingCommand` grammar the CLI
//! uses — `/job` is `saya job`, `/start` is `saya start`, `/follow-up` is
//! `saya follow-up` — so the surfaces cannot drift.

use super::{SlashCommand, SlashParseError};
use crate::cli::CodingCommand;
use clap::Parser;

/// The grammar one spelling states. `--tools` is global so it may follow
/// `start`, the one verb that reads it; every other spelling refuses it.
#[derive(Parser)]
struct JobArgs {
    /// Advertise only these model tools to this job and its helpers,
    /// comma-separated and repeatable. Inside a session this may only narrow
    /// the tools the session itself advertises; the launch spelling (`saya
    /// --tools … start …`) is the same one global list, resolved into the
    /// config before the job starts.
    #[arg(long, value_name = "NAME,...", value_delimiter = ',', global = true)]
    tools: Vec<String>,
    #[command(subcommand)]
    command: CodingCommand,
}

/// One spelling's tail, bounded before it reaches the tokenizer or clap.
const BYTE_BOUND: usize = 32 * 1024;

/// `verb` is the subcommand word the spelling itself names (`start` for
/// `/start`); `/job` states its own verb in the tail, so it passes `None`.
fn parse_words(program: &str, verb: Option<&str>, tail: &str) -> Result<JobArgs, SlashParseError> {
    if tail.len() > BYTE_BOUND {
        return Err(SlashParseError(format!(
            "{program} command exceeds its byte bound"
        )));
    }
    let words = crate::interactive::session::run::tokenize_tail(tail).map_err(SlashParseError)?;
    let verb = verb.map(str::to_string);
    JobArgs::try_parse_from(
        std::iter::once(program.to_string())
            .chain(verb)
            .chain(words),
    )
    .map_err(|error| SlashParseError(error.to_string()))
}

/// A spelling that does not read `--tools` refuses a stated list rather than
/// parsing and dropping it, the way the CLI's dispatch guard does.
fn refuse_tools(args: JobArgs) -> Result<CodingCommand, SlashParseError> {
    if args.tools.is_empty() {
        Ok(args.command)
    } else {
        Err(SlashParseError(crate::cli::TOOLS_REFUSAL.to_owned()))
    }
}

/// `/start <id> <task> …` — `saya start` inside the session, including the
/// session's own `--tools` narrowing words.
pub(super) fn parse_start(tail: &str) -> Result<SlashCommand, SlashParseError> {
    let mut args = parse_words("start", Some("start"), tail)?;
    let stated = std::mem::take(&mut args.tools);
    let CodingCommand::Start { tools, .. } = &mut args.command else {
        return Err(SlashParseError("start requires a start command".into()));
    };
    *tools = stated;
    Ok(SlashCommand::Coding(args.command))
}

/// `/follow-up <id> <agent> --expected-completion-sequence <n> <instruction>`
/// — `saya follow-up` inside the session.
pub(super) fn parse_follow_up(tail: &str) -> Result<SlashCommand, SlashParseError> {
    refuse_tools(parse_words("follow-up", Some("follow-up"), tail)?).map(SlashCommand::Coding)
}

/// `/approve`, `/deny`, `/message`, `/pause`, `/continue`, `/cancel-child` —
/// the six controls that only a live session can answer.
pub(super) fn parse_control(program: &str, tail: &str) -> Result<SlashCommand, SlashParseError> {
    refuse_tools(parse_words(program, Some(program), tail)?).map(SlashCommand::Coding)
}

/// `/job <verb>` — the lifecycle command, the session mirror of `saya job`. An
/// agent-shaped verb is refused by name, naming the spelling that works.
pub(super) fn parse_job(tail: &str) -> Result<SlashCommand, SlashParseError> {
    let command = refuse_tools(parse_words("job", None, tail)?)?;
    if let Some(spelling) = session_spelling(&command) {
        return Err(SlashParseError(format!(
            "`/job` runs the durable-job verbs; this one has its own spelling: `{spelling}`"
        )));
    }
    Ok(SlashCommand::Coding(command))
}

/// The session spelling of an agent-shaped verb. The six controls share the
/// CLI command's map; the two launches name their session spelling.
fn session_spelling(command: &CodingCommand) -> Option<&'static str> {
    match command {
        CodingCommand::Start { .. } => Some("/start <id> <task> …"),
        CodingCommand::FollowUp { .. } => Some("/follow-up <id> <agent> …"),
        other => crate::cli::job_session_spelling(other),
    }
}
