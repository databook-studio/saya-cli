//! Which family a call belongs to, the title the panel puts up, and the place
//! line it opens with.
//!
//! A family is read from the call's own facts — the shape the one operator
//! builds for every caller (`chat::operator`) — never from a tool name spelled
//! a second time here: a shell's facts open with the directory it runs in, a
//! query's with the connection, a write's with the path, a fetch's with the
//! request. The plan gate's ask is the one body that is free text, so it is the
//! one the caller's line identifies.
//!
//! How a place is shortened for the panel's width is [`super::cards_place`]'s;
//! this module says what each family's place is.

use super::cards::Family;
use super::cards_place::Place;
use crate::agent::tools::coding::plan_gate;
use crate::agent::tools::consent::Caller;
use crate::chat::Question;

/// What the panel asks, per family: the sentence an operator answers.
const COMMAND: &str = "Allow this command?";
const FILE_CHANGE: &str = "Allow this file change?";
const QUERY: &str = "Allow this query?";
const WEB: &str = "Allow this web request?";
const PLAN: &str = "Approve this plan?";
const SCOPE: &str = "Approve this job?";
const OTHER: &str = "Allow this call?";

/// The lane fact a command's place states: a host command runs outside the
/// sandbox, and the operator deciding on it has to know. It is a safety fact,
/// so it is never dropped from the place — it moves to a row of its own when
/// the line has no room for it.
const NOT_SANDBOXED: &str = "on your machine, not sandboxed";

/// What a workspace write's place states beside the folder it lands in.
const IN_WORKSPACE: &str = "inside the workspace";

/// The family `question` belongs to.
pub(crate) fn family(question: &Question) -> Family {
    let first = question.facts.lines().next().unwrap_or_default();
    if first.starts_with("in: ") {
        Family::Command
    } else if first.starts_with("on: ") {
        Family::Query
    } else if first.starts_with("path: ") {
        Family::FileChange
    } else if first.starts_with("GET ") {
        Family::Web
    } else if question.caller.contains(&plan_gate::line(&Caller::Session)) {
        Family::Plan
    } else if first.starts_with("Approve job ") {
        Family::Scope
    } else {
        Family::Other
    }
}

/// The panel's title: what is being allowed, and — with more than one waiting —
/// which of them this is.
pub(crate) fn title(family: Family, waiting: usize) -> String {
    let asked = match family {
        Family::Command => COMMAND,
        Family::FileChange => FILE_CHANGE,
        Family::Query => QUERY,
        Family::Web => WEB,
        Family::Plan => PLAN,
        Family::Scope => SCOPE,
        Family::Other => OTHER,
    };
    if waiting > 1 {
        format!("{asked} · 1 of {waiting}")
    } else {
        asked.to_owned()
    }
}

/// Where the call runs: the folder and the lane for a command, the workspace
/// for a write, the connection for a query, this machine for a fetch.
///
/// A shell reaches the operator only on the host lane — the policy asks on the
/// sandbox lane's behalf only to reroute the call, never to allow it
/// (`shell_policy::policy::lane`) — so the panel states what the ask means.
/// `in_facts` is the place the call's own facts name; `folder` is the launch's
/// workspace, where one is bound.
pub(crate) fn place(
    family: Family,
    in_facts: Option<String>,
    folder: Option<&str>,
) -> Option<Place> {
    match family {
        Family::Command => {
            let place = command_place(in_facts.as_deref(), folder)?;
            Some(place.noting(NOT_SANDBOXED))
        }
        Family::FileChange => Some(match folder {
            Some(folder) => Place::in_folder(folder, Some(IN_WORKSPACE)),
            None => Place::words(IN_WORKSPACE),
        }),
        Family::Query => in_facts.map(Place::words),
        Family::Web => Some(Place::words("from this machine")),
        Family::Scope => folder.map(|folder| Place::in_folder(folder, None)),
        Family::Plan | Family::Other => None,
    }
}

/// Where a command runs, as the place line names it: the directory the call's
/// own facts state, or — when it states the default one (`.`) or none — the
/// launch's folder. The line names the folder, never the spelling the call used,
/// and `.` names nothing at all.
fn command_place(in_facts: Option<&str>, folder: Option<&str>) -> Option<Place> {
    match in_facts {
        Some(statement) if !is_bare(statement) => Some(Place::in_folder(
            statement.strip_prefix("in ").unwrap_or(statement),
            None,
        )),
        _ => folder.map(|folder| Place::in_folder(folder_name(folder), None)),
    }
}

/// Whether a place statement names no directory of its own: the default `.`,
/// or nothing after the `in `.
fn is_bare(statement: &str) -> bool {
    let directory = statement.strip_prefix("in ").unwrap_or(statement).trim();
    directory.is_empty() || directory == "."
}

/// The folder's own name: its last path component, so `/home/me/demo` reads as
/// `demo`. A path with no component of its own (`/`) stays as it is spelled,
/// because there is no shorter name for it that says more.
fn folder_name(folder: &str) -> String {
    std::path::Path::new(folder).file_name().map_or_else(
        || folder.to_owned(),
        |name| name.to_string_lossy().into_owned(),
    )
}
