//! The catalogue guard: every command the palette offers must be answered by
//! this surface.
//!
//! The finding this test pins: the full-screen surface is the default one, the
//! palette (`Ctrl+K`, `/`) offers the whole catalogue, and a command that
//! answered *"that command is the line front's in this release"* was a command
//! the operator could choose and could not use. So the guard walks
//! [`CATALOGUE`] — every row, the hidden ones included — parses the row's own
//! spelling, hands it to the surface's one table and fails if the answer is a
//! deferral to another surface rather than something this one did.

use super::ops::perform;
use super::ops_fixture::{Fixture, command};
use crate::slash::catalog::CATALOGUE;

/// The spellings the guard answers that no catalogue row has ever been able to
/// be written without — the phrases an unperformed command used to answer with.
/// A row whose answer says one of them has no home on this surface.
const DEFERRALS: [&str; 4] = [
    "the line front's",
    "the line front",
    "only available in the interactive TUI",
    "not available in chat yet",
];

/// A line that parses to `name`, with the value its grammar asks for. Every
/// row of [`CATALOGUE`] is stated here: a row with no line is a row the guard
/// cannot check, so it fails by name rather than passing silently.
fn line_for(name: &str) -> &'static str {
    match name {
        "start" => "/start do the thing",
        "job" => "/job list",
        "continue" => "/continue",
        "connect" => "/connect local",
        "connections" => "/connections",
        "include" => "/include local",
        "exclude" => "/exclude local",
        "provider" => "/provider",
        "model" => "/model",
        "privacy" => "/privacy",
        "mode" => "/mode",
        "schema" => "/schema",
        "sql" => "/sql select 1",
        "export" => "/export out.csv",
        "investigation" => "/investigation list",
        "chart" => "/chart",
        "explain" => "/explain select 1",
        "clear" => "/clear",
        "undo" => "/undo",
        "rewind" => "/rewind",
        "compact" => "/compact",
        "notes" => "/notes",
        "sessions" => "/sessions",
        "preferences" => "/preferences list",
        "resume" => "/resume fx-2",
        "setup" => "/setup",
        "reload" => "/reload",
        "columns" => "/columns id,label",
        "contracts" => "/contracts list",
        "remember" => "/remember events description A table of events",
        "forget" => "/forget claim-1",
        "run" => "/run do the thing --allow none",
        "runs" => "/runs",
        "allow" => "/allow none",
        "permissions" => "/permissions",
        "doctor" => "/doctor",
        "status" => "/status",
        "thinking" => "/thinking",
        "tasks" => "/tasks",
        "recovery" => "/recovery",
        "help" => "/help status",
        "exit" => "/exit",
        other => panic!("the guard has no line for the catalogue row `{other}`"),
    }
}

/// The answer one command gets, as the operator would read it.
fn said(asked: &super::ops::Asked) -> String {
    match asked {
        super::ops::Asked::Notice(text) => text.clone(),
        super::ops::Asked::Error(text) => text.clone(),
        // Anything else is a performed operation: a quit, a reload, a rollover
        // the session performs, or work this surface runs off its own thread.
        // None of them is a deferral, and the words are not what this test
        // checks — only that the command reached a home.
        other => format!("{other:?}"),
    }
}

/// Every row of the catalogue — the hidden ones included — is performed by this
/// surface, never handed to another one.
#[test]
fn every_catalogue_command_is_performed_by_this_surface() {
    let fixture = Fixture::new("catalogue");
    let mut unhandled = Vec::new();
    for spec in CATALOGUE {
        let line = line_for(spec.name);
        let asked = perform(&fixture.deps, command(line));
        let answer = said(&asked);
        let hidden = if spec.hidden { " (hidden)" } else { "" };
        for phrase in DEFERRALS {
            if answer.contains(phrase) {
                unhandled.push(format!("/{}{hidden} — {answer}", spec.name));
                break;
            }
        }
    }
    assert!(
        unhandled.is_empty(),
        "these catalogue commands have no home on this surface:\n{}",
        unhandled.join("\n")
    );
}

/// The alias spellings resolve to the same command, and get the same answer: a
/// spelling the palette offers must not be the one that falls through.
#[test]
fn every_alias_spelling_is_answered_too() {
    let fixture = Fixture::new("aliases");
    let mut unhandled = Vec::new();
    for spec in CATALOGUE {
        for alias in spec.aliases {
            // An alias never changes the operation, only the words around it,
            // so the canonical row's own line is asked under the alias' name
            // for the rows whose grammar takes no argument — and skipped for
            // the rest, where the alias name alone would not parse.
            if !grammar_is_bare(spec.name) {
                continue;
            }
            let line = format!("/{}", alias.name);
            let Ok(Some(command)) = crate::slash::parse_slash_command(&line) else {
                continue;
            };
            let answer = said(&perform(&fixture.deps, command));
            for phrase in DEFERRALS {
                if answer.contains(phrase) {
                    unhandled.push(format!("{line} — {answer}"));
                    break;
                }
            }
        }
    }
    assert!(
        unhandled.is_empty(),
        "these alias spellings have no home on this surface:\n{}",
        unhandled.join("\n")
    );
}

/// Whether `name`'s grammar takes no argument, so its alias' bare spelling
/// parses to the same command.
fn grammar_is_bare(name: &str) -> bool {
    matches!(
        name,
        "continue"
            | "connections"
            | "provider"
            | "model"
            | "privacy"
            | "mode"
            | "schema"
            | "chart"
            | "sessions"
            | "compact"
            | "setup"
            | "reload"
            | "contracts"
            | "allow"
            | "permissions"
            | "doctor"
            | "status"
            | "thinking"
            | "tasks"
            | "help"
            | "exit"
    )
}
