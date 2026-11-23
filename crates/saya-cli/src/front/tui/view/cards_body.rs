//! The call's own block: the bytes an operator answers for, and the place the
//! facts name inside them.
//!
//! The operator's facts are one shape per family (`chat::operator`): a command
//! opens with the directory it runs in, a query with the connection, a write
//! with the path, a fetch with the request. This module takes that opening line
//! out of the block — it becomes the panel's place line — and leaves the rest
//! exactly as it was written, so no byte the operator approves is reworded.
//!
//! A job's consent states one machine line after its human ones: the proposal
//! itself, as JSON. It is what the yes covers, and it is unreadable at a glance,
//! so it is lifted out of the block into the card's `v` details rather than
//! crowding the four lines an operator actually decides on.

use super::cards::Family;
use crate::interactive::tui::wrap::wrap_cells;

/// One card's body split into the block and the place its facts named.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Parts {
    /// The call itself: the command, the statement, the path, the request.
    pub(crate) target: Vec<String>,
    /// The call's machine bytes — a job's proposal line — which the panel holds
    /// behind `v`.
    pub(crate) details: Vec<String>,
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
                details: Vec::new(),
                in_facts: Some(format!("in {directory}")),
            },
            _ => whole(&lines),
        },
        Family::Query => match first.strip_prefix("on: ") {
            Some(connection) if !rest().is_empty() => Parts {
                target: rest(),
                details: Vec::new(),
                in_facts: Some(format!("on connection {connection}")),
            },
            _ => whole(&lines),
        },
        Family::FileChange => match first.strip_prefix("path: ") {
            Some(path) => Parts {
                target: vec![path.to_owned()],
                details: Vec::new(),
                in_facts: None,
            },
            None => whole(&lines),
        },
        Family::Scope => machine_lines(&lines),
        Family::Web | Family::Plan | Family::Other => whole(&lines),
    }
}

/// `lines` as body rows, with the machine lines — the proposal a job's consent
/// ends in — lifted into the details `v` opens.
fn whole(lines: &[&str]) -> Parts {
    Parts {
        target: lines.iter().map(|line| (*line).to_owned()).collect(),
        details: Vec::new(),
        in_facts: None,
    }
}

/// `lines` split at the machine lines a consent states after its human ones: a
/// JSON object is the exact proposal, and it belongs behind `v`.
fn machine_lines(lines: &[&str]) -> Parts {
    let (details, target): (Vec<&str>, Vec<&str>) = lines
        .iter()
        .partition(|line| line.trim_start().starts_with('{'));
    Parts {
        target: target.iter().map(|line| (*line).to_owned()).collect(),
        details: details.iter().map(|line| (*line).to_owned()).collect(),
        in_facts: None,
    }
}

/// `rows` wrapped to `room` cells: no row of a call is ever cut, so what the
/// operator reads is what they are answering for, continued on the next row.
pub(crate) fn wrap(rows: &[String], room: usize) -> Vec<String> {
    rows.iter()
        .flat_map(|row| wrap_cells(row, room.max(1)))
        .collect()
}
