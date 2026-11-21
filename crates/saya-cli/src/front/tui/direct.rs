//! The slash commands the surface answers itself (`03` §4.1): `/sql`,
//! `/export`, `/chart` and `/investigation`.
//!
//! Two halves, both pure. [`read`] reads one submitted line through the
//! crate's one slash parser and answers what the line comes to: a [`Cmd`] the
//! runtime performs, a sentence the surface says instead, or nothing at all —
//! the line is a turn's. [`land`] files what a command handed back into the
//! transcript: a query's rows as a result entry (`02` §3.3, T6's preview and
//! table), a command's own sentence as a notice (`02` §3.10), a failure as the
//! error block (`02` §3.7). Nothing here runs a query, writes a file or paints
//! a row.
//!
//! What the commands do lives beside this file: [`super::direct_run`] performs
//! them off the loop's thread — `/sql` through today's bounded, read-only
//! worker — and [`super::direct_trust`] holds the workspace trust answer.

use saya_types::QueryResult;

use super::model::Model;
use super::msg::Cmd;
use crate::front::view::{Entry, Error, Notice, ResultPreview};
use crate::interactive::tui::LastQuery;
use crate::slash::{SlashCommand, parse_slash_command};

/// What `/export` says with no result to write (`03` §4.1).
pub(crate) const NOTHING_TO_EXPORT: &str = "Nothing to export yet — run a query first.";

/// What `/chart` says with no result to chart, in today's words.
pub(crate) const NOTHING_TO_CHART: &str = "Nothing to chart yet — run a query first.";

/// One command the surface reads as its own, already resolved against the
/// state the surface holds: the run's connection, and the result to write.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Direct {
    /// `/sql <query>`: run the query through today's bounded read-only worker,
    /// on the connection the run is on.
    Sql {
        profile: Option<String>,
        sql: String,
    },
    /// `/export <path>`: write this result through today's export.
    Export {
        path: String,
        overwrite: bool,
        result: QueryResult,
    },
    /// `/chart [kind] [path]`: chart this result through today's renderer.
    Chart { args: String, result: QueryResult },
    /// `/investigation …`: the crate's own parsed command, and the query a
    /// `save` with no `--sql` is filled from.
    Investigation {
        command: crate::cli::InvestigationCommand,
        last_query: Option<LastQuery>,
    },
}

/// What a direct command handed back.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum DirectOutcome {
    /// The rows a query returned: a result entry (`02` §3.3).
    Result(ResultPreview),
    /// One line the surface says in the transcript, in its own voice
    /// (`02` §3.10).
    Notice(String),
    /// What ended a direct command in failure (`02` §3.7).
    Error(String),
}

/// What one submitted line comes to.
pub(crate) enum Read {
    /// Not the surface's own command: a turn's line.
    Turn,
    /// One command the runtime performs.
    Cmd(Cmd),
    /// The words the line is answered with instead of running.
    Said(String),
}

/// Reads one submitted line as a direct command, or leaves it to a turn.
pub(crate) fn read(model: &Model, line: &str) -> Read {
    match parse_slash_command(line) {
        Ok(Some(SlashCommand::Sql(sql))) => Read::Cmd(Cmd::Direct(Direct::Sql {
            profile: model.database_name().map(str::to_owned),
            sql,
        })),
        Ok(Some(SlashCommand::Export(request))) => match target(model) {
            Some(result) => Read::Cmd(Cmd::Direct(Direct::Export {
                path: request.path,
                overwrite: request.overwrite,
                result,
            })),
            None => Read::Said(NOTHING_TO_EXPORT.to_owned()),
        },
        Ok(Some(SlashCommand::Chart(args))) => match target(model) {
            Some(result) => Read::Cmd(Cmd::Direct(Direct::Chart { args, result })),
            None => Read::Said(NOTHING_TO_CHART.to_owned()),
        },
        Ok(Some(SlashCommand::Investigation(command))) => {
            Read::Cmd(Cmd::Direct(Direct::Investigation {
                command,
                last_query: last_query(model),
            }))
        }
        // Every other line — a message, and a command whose argument is a
        // later unit's — is still a turn's (T18's rule for `/mode plan`).
        Ok(_) => Read::Turn,
        // A malformed command is the surface's own answer: it says what the
        // parser said rather than sending the line to the model.
        Err(error) => Read::Said(error.to_string()),
    }
}

/// Files what a direct command handed back at its place in the newest turn.
pub(crate) fn land(model: &mut Model, outcome: DirectOutcome) {
    let entry = match outcome {
        DirectOutcome::Result(preview) => Entry::ResultPreview(preview),
        DirectOutcome::Notice(message) => Entry::Notice(Notice { message }),
        DirectOutcome::Error(message) => Entry::Error(Error { message }),
    };
    model.transcript.current().push(entry);
}

/// The result `/export` and `/chart` write: the one the cursor holds (T15)
/// when it holds a result, else the newest result the conversation shows.
fn target(model: &Model) -> Option<QueryResult> {
    if let Some(Entry::ResultPreview(preview)) = model.focused_entry()
        && preview.refusal.is_none()
    {
        return Some(preview.result.clone());
    }
    model
        .turns()
        .iter()
        .rev()
        .flat_map(|turn| turn.entries().iter().rev())
        .find_map(|entry| match entry {
            Entry::ResultPreview(preview) if preview.refusal.is_none() => {
                Some(preview.result.clone())
            }
            _ => None,
        })
}

/// The last query the conversation shows, as today's `/investigation save`
/// fill reads it: the SQL it ran and the connection that ran it.
fn last_query(model: &Model) -> Option<LastQuery> {
    model
        .turns()
        .iter()
        .rev()
        .flat_map(|turn| turn.entries().iter().rev())
        .find_map(|entry| match entry {
            Entry::ResultPreview(preview) if preview.refusal.is_none() => Some(LastQuery {
                sql: preview.sql.clone(),
                connection: Some(preview.connection.clone()),
            }),
            _ => None,
        })
}
