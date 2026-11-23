//! The call's own block: the bytes an operator answers for, and the place the
//! facts name inside them.
//!
//! The operator's facts are one shape per family (`chat::operator`): a command
//! opens with the directory it runs in, a query with the connection, a write
//! with the path, a fetch with the request. This module takes that opening line
//! out of the block — it becomes the panel's place line — and leaves the rest
//! exactly as it was written, so no byte the operator approves is reworded.

use super::cards::Family;
use crate::interactive::tui::wrap::wrap_cells;

/// One card's body split into the block and the place its facts named.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Parts {
    /// The call itself: the command, the statement, the path, the request.
    pub(crate) target: Vec<String>,
    /// Where it runs, as the facts state it (`in ~/shop`, `on connection demo`).
    pub(crate) in_facts: Option<String>,
}

/// The parts of `facts`, read as `family`.
pub(crate) fn parts(facts: &str, family: Family) -> Parts {
    let lines: Vec<&str> = facts.split('\n').collect();
    let first = lines.first().copied().unwrap_or_default();
    let rest = || {
        lines[1.min(lines.len())..]
            .iter()
            .map(|line| (*line).to_owned())
            .collect::<Vec<_>>()
    };
    match family {
        Family::Command => match first.strip_prefix("in: ") {
            // A command whose facts name no directory is the whole block.
            Some(directory) if !rest().is_empty() => Parts {
                target: rest(),
                in_facts: Some(format!("in {directory}")),
            },
            _ => Parts {
                target: whole(&lines),
                in_facts: None,
            },
        },
        Family::Query => match first.strip_prefix("on: ") {
            Some(connection) if !rest().is_empty() => Parts {
                target: rest(),
                in_facts: Some(format!("on connection {connection}")),
            },
            _ => Parts {
                target: whole(&lines),
                in_facts: None,
            },
        },
        Family::FileChange => match first.strip_prefix("path: ") {
            Some(path) => Parts {
                target: vec![path.to_owned()],
                in_facts: None,
            },
            None => Parts {
                target: whole(&lines),
                in_facts: None,
            },
        },
        Family::Web | Family::Plan | Family::Scope | Family::Other => Parts {
            target: whole(&lines),
            in_facts: None,
        },
    }
}

/// `lines` as body rows, verbatim.
fn whole(lines: &[&str]) -> Vec<String> {
    lines.iter().map(|line| (*line).to_owned()).collect()
}

/// `rows` wrapped to `room` cells: no row of a call is ever cut, so what the
/// operator reads is what they are answering for, continued on the next row.
pub(crate) fn wrap(rows: &[String], room: usize) -> Vec<String> {
    rows.iter()
        .flat_map(|row| wrap_cells(row, room.max(1)))
        .collect()
}
