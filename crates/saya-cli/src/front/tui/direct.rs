//! The slash commands the surface answers itself (`03` §4.1): `/sql`,
//! `/export`, `/chart` and `/investigation` — and, since the harness run engine
//! is not this surface's (O5, K8), one pointer line for `/run …`, `/run cancel
//! …` and `/runs`.
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

use std::sync::Arc;

use saya_types::{ExecutionEvidence, QueryResult};

use super::model::Model;
use super::msg::Cmd;
use crate::front::view::{Entry, Error, Notice, ResultPreview};
use crate::interactive::tui::LastQuery;
use crate::slash::{ExportRequest, SlashCommand, parse_slash_command};

/// What `/export` says with no result to write (`03` §4.1).
pub(crate) const NOTHING_TO_EXPORT: &str = "Nothing to export yet — run a query first.";

/// What `/chart` says with no result to chart, in today's words.
pub(crate) const NOTHING_TO_CHART: &str = "Nothing to chart yet — run a query first.";

/// What the surface says to `/run …`, `/run cancel …` and `/runs` (O5): the
/// harness run engine is not this surface's, and the pointer names the terminal
/// command that drives it. The line REPL keeps today's behavior.
pub(crate) const RUN_MOVED: &str = "/run moved out of chat — use `saya run` in a terminal";

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
    /// `/export <path>`: write this result through today's export — the
    /// Markdown report when the request names one, the data writer otherwise.
    Export {
        /// The one parsed `/export` request: the format the destination's
        /// extension or `--format` names, its `--rows` bound, and whether an
        /// existing file may be replaced.
        request: ExportRequest,
        result: QueryResult,
        /// The execution's provenance: what the report states, and what the
        /// line REPL's own report reads from its capture.
        evidence: Option<Arc<ExecutionEvidence>>,
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
            Some((result, evidence)) => Read::Cmd(Cmd::Direct(Direct::Export {
                request,
                result,
                evidence,
            })),
            None => Read::Said(no_result(model)),
        },
        Ok(Some(SlashCommand::Chart(args))) => match target(model) {
            Some((result, _)) => Read::Cmd(Cmd::Direct(Direct::Chart { args, result })),
            None => Read::Said(NOTHING_TO_CHART.to_owned()),
        },
        Ok(Some(SlashCommand::Investigation(command))) => {
            Read::Cmd(Cmd::Direct(Direct::Investigation {
                command,
                last_query: last_query(model),
            }))
        }
        // O5: the harness run engine is not this surface's, so every spelling
        // of it — `/run …`, `/run cancel …`, `/runs` — is one pointer line to
        // the terminal command that does drive it. A hidden command elsewhere
        // still resolves and runs: the hide is the catalogue's, not this
        // match's.
        Ok(Some(SlashCommand::Run(_) | SlashCommand::RunCancel(_) | SlashCommand::Runs(_))) => {
            Read::Said(RUN_MOVED.to_owned())
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
/// when it holds a result, else the newest result the conversation shows —
/// with the execution's provenance, when it has one.
fn target(model: &Model) -> Option<(QueryResult, Option<Arc<ExecutionEvidence>>)> {
    let preview = shown(model)?
        .into_iter()
        .find(|preview| preview.refusal.is_none())?;
    Some((preview.result.clone(), preview.evidence.clone()))
}

/// What `/export` says with no held result: the newest capture's refusal in
/// its own words when the surface holds one, else that there is nothing yet.
fn no_result(model: &Model) -> String {
    let refused = shown(model).and_then(|previews| previews.into_iter().find_map(|p| p.refusal));
    match refused {
        Some(reason) => format!(
            "Nothing to export: {}.",
            crate::front::tui::view::refusal_words(reason)
        ),
        None => NOTHING_TO_EXPORT.to_owned(),
    }
}

/// The results the surface shows, the focused one first and then the
/// conversation's newest first — the order `/export` and `/chart` read.
fn shown(model: &Model) -> Option<Vec<&ResultPreview>> {
    let mut previews = Vec::new();
    if let Some(Entry::ResultPreview(preview)) = model.focused_entry() {
        previews.push(preview);
    }
    let newest = model
        .turns()
        .iter()
        .rev()
        .flat_map(|turn| turn.entries().iter().rev())
        .filter_map(|entry| match entry {
            Entry::ResultPreview(preview) => Some(preview),
            _ => None,
        });
    previews.extend(newest);
    (!previews.is_empty()).then_some(previews)
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
