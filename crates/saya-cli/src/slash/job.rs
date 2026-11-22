//! The session's job surface: `/job <verb>` for the durable-job lifecycle and
//! its child controls, `/start` for the one launch with a spelling of its own,
//! and the seven top-level spellings `/job`'s verbs still answer to for one
//! release (`/approve`, `/deny`, `/message`, `/pause`, `/continue <id>
//! <agent>`, `/cancel-child`, `/follow-up`).
//!
//! Every spelling builds the one `CodingCommand` the CLI carries, so the
//! surfaces cannot drift: `/job`, `/follow-up` and the controls parse through
//! its clap grammar, while `/start` reads its own line so the task can be the
//! whole remaining text and the id can be generated. A deprecated spelling also
//! records its alias (`slash::JobRequest`), which is the note it owes once per
//! session.

use super::{JobAlias, JobRequest, SlashCommand, SlashParseError};
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

/// `/start [--id <id>] <task> [flags]` — `saya start` inside the session.
///
/// The task is the whole remaining text: quotes group it and every
/// recognised flag is removed wherever it appears. The id is generated unless
/// `--id` states one, so starting ordinary work needs no id to type. The old
/// positional `id task` form is gone — an unquoted first word is ambiguous
/// with a task, so it is read as task text, never as an id. `--` ends the
/// flags, for task text that itself begins with `--`.
pub(super) fn parse_start(tail: &str) -> Result<SlashCommand, SlashParseError> {
    if tail.len() > BYTE_BOUND {
        return Err(SlashParseError(
            "start command exceeds its byte bound".into(),
        ));
    }
    let words = crate::interactive::session::run::tokenize_tail(tail).map_err(SlashParseError)?;
    let mut start = StartWords::default();
    let mut task = Vec::new();
    let mut literal = false;
    let mut index = 0;
    while let Some(word) = words.get(index) {
        index += 1;
        if literal {
            task.push(word.clone());
            continue;
        }
        if word == "--" {
            literal = true;
            continue;
        }
        let Some(flag) = word.strip_prefix("--") else {
            task.push(word.clone());
            continue;
        };
        let (name, inline) = flag
            .split_once('=')
            .map_or((flag, None), |(name, value)| (name, Some(value)));
        match name {
            "id" => {
                let value = start.value(&words, &mut index, inline, name)?;
                saya_types::JobId::parse(&value).map_err(|_| {
                    SlashParseError(format!(
                        "`--id` must be 1-128 letters, digits, `-` or `_`: {value}"
                    ))
                })?;
                if start.id.replace(value).is_some() {
                    return Err(SlashParseError("`--id` is stated more than once".into()));
                }
            }
            "write" => {
                let value = start.value(&words, &mut index, inline, name)?;
                start.write_paths.push(value);
            }
            "database-profile" => {
                let value = start.value(&words, &mut index, inline, name)?;
                start.database_profiles.push(value);
            }
            "tools" => {
                let value = start.value(&words, &mut index, inline, name)?;
                start.tools.extend(
                    value
                        .split(',')
                        .filter(|name| !name.is_empty())
                        .map(str::to_owned),
                );
            }
            "plan" | "in-place" | "managed" | "approve" => {
                if inline.is_some() {
                    return Err(SlashParseError(format!("`--{name}` takes no value")));
                }
                start.set_flag(name);
            }
            other => {
                return Err(SlashParseError(format!(
                    "unknown flag `--{other}`: put task text that begins with `--` after `--`"
                )));
            }
        }
    }
    if start.in_place && start.managed {
        return Err(SlashParseError(
            "`--in-place` and `--managed` cannot be stated together".into(),
        ));
    }
    if task.is_empty() {
        return Err(SlashParseError(
            "start requires a task: /start <task>".into(),
        ));
    }
    let id = start
        .id
        .unwrap_or_else(|| crate::commands::coding::generate_job_id().to_string());
    Ok(SlashCommand::Coding(JobRequest::job(
        CodingCommand::Start {
            id,
            task: task.join(" "),
            write_paths: start.write_paths,
            database_profiles: start.database_profiles,
            tools: start.tools,
            plan: start.plan,
            approve: start.approve,
            in_place: start.in_place,
            managed: start.managed,
        },
    )))
}

/// The words a `/start` line states besides its task, before the command is
/// built.
#[derive(Default)]
struct StartWords {
    id: Option<String>,
    write_paths: Vec<String>,
    database_profiles: Vec<String>,
    tools: Vec<String>,
    plan: bool,
    approve: bool,
    in_place: bool,
    managed: bool,
}

impl StartWords {
    /// The value of `--flag value` or `--flag=value`, consumed from `index`.
    fn value(
        &self,
        words: &[String],
        index: &mut usize,
        inline: Option<&str>,
        flag: &str,
    ) -> Result<String, SlashParseError> {
        if let Some(value) = inline {
            if value.is_empty() {
                return Err(SlashParseError(format!("`--{flag}` needs a value")));
            }
            return Ok(value.to_owned());
        }
        let value = words
            .get(*index)
            .ok_or_else(|| SlashParseError(format!("`--{flag}` needs a value")))?;
        *index += 1;
        Ok(value.clone())
    }

    fn set_flag(&mut self, name: &str) {
        match name {
            "plan" => self.plan = true,
            "approve" => self.approve = true,
            "in-place" => self.in_place = true,
            "managed" => self.managed = true,
            _ => {}
        }
    }
}

/// `/follow-up <id> <agent> --expected-completion-sequence <n> <instruction>`
/// — `/job follow-up`'s one-release top-level spelling.
pub(super) fn parse_follow_up(tail: &str) -> Result<SlashCommand, SlashParseError> {
    let command = refuse_tools(parse_words("follow-up", Some("follow-up"), tail)?)?;
    Ok(SlashCommand::Coding(JobRequest::alias(
        command,
        JobAlias::FollowUp,
    )))
}

/// `/approve`, `/deny`, `/message`, `/pause`, `/cancel-child` — the top-level
/// spellings of `/job`'s own verbs, for one release.
pub(super) fn parse_control(program: &str, tail: &str) -> Result<SlashCommand, SlashParseError> {
    let command = refuse_tools(parse_words(program, Some(program), tail)?)?;
    let alias = match program {
        "approve" => JobAlias::Approve,
        "deny" => JobAlias::Deny,
        "message" => JobAlias::Message,
        "pause" => JobAlias::Pause,
        "continue" => JobAlias::Continue,
        other => {
            debug_assert_eq!(other, "cancel-child", "the six controls are named");
            JobAlias::CancelChild
        }
    };
    Ok(SlashCommand::Coding(JobRequest::alias(command, alias)))
}

/// `/continue` — bare continues the session's paused turn; with an id and an
/// agent it is `/job continue`'s one-release top-level spelling.
pub(super) fn parse_continue(tail: &str) -> Result<SlashCommand, SlashParseError> {
    if tail.trim().is_empty() {
        return Ok(SlashCommand::ContinueTurn);
    }
    let words = crate::interactive::session::run::tokenize_tail(tail).map_err(SlashParseError)?;
    if words.len() < 2 {
        return Err(SlashParseError(
            "continue needs an id and an agent: `/job continue <id> <agent>` continues a \
             child, and bare `/continue` continues a paused turn"
                .into(),
        ));
    }
    parse_control("continue", tail)
}

/// `/job <verb>` — the lifecycle command, the session mirror of `saya job`.
/// The lifecycle verbs run here; a launch that has its own spelling is refused
/// by name, naming the spelling that works.
pub(super) fn parse_job(tail: &str) -> Result<SlashCommand, SlashParseError> {
    let command = refuse_tools(parse_words("job", None, tail)?)?;
    if let Some(spelling) = session_spelling(&command) {
        return Err(SlashParseError(format!(
            "`/job` runs the durable-job verbs; this one has its own spelling: `{spelling}`"
        )));
    }
    Ok(SlashCommand::Coding(JobRequest::job(command)))
}

/// The session spelling of a verb that is not `/job`'s own. Only the two
/// launches are left: every child control is a `/job` verb now, and its
/// top-level spelling is a one-release alias of it (`ref/03-commands.md` row 171).
fn session_spelling(command: &CodingCommand) -> Option<&'static str> {
    match command {
        CodingCommand::Start { .. } => Some("/start <task>"),
        _ => None,
    }
}
