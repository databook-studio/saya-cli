//! The palette's ranking: a query in, the catalogue rows it matches out.
//!
//! The palette keeps no command list of its own. Every row is read from
//! [`catalog::CATALOGUE`], filtered to the rows the `/help` listing carries
//! (`listed`) and the TUI can run (`surfaces`), and scored with the same fuzzy
//! matcher the `/` popup uses, so a name, a description or a surface never has
//! a second definition to drift from.
//!
//! A query is matched against every spelling a command answers to: the
//! canonical name first, then its aliases. An alias that scores *better* than
//! the canonical name ranks the canonical command and is remembered, so the
//! pane can say which spelling found it — an operator who types `/approvals`
//! is shown `/mode`, because `/mode` is what the command is called.

use std::cmp::Reverse;

use crate::interactive::tui::fuzzy_score;
use crate::slash::catalog::{self, Category, CommandSpec, Surface};

/// One palette row: a catalogued command, the spelling that matched it, and
/// what the list row and the usage row say about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Entry {
    /// The canonical name, without the `/`.
    pub(crate) name: &'static str,
    /// The argument synopsis the `/help` listing prints.
    pub(crate) usage: &'static str,
    /// The catalogue's one-line description.
    pub(crate) summary: &'static str,
    /// The heading the `/help` listing files it under.
    pub(crate) category: Category,
    /// The alias spelling that matched, when the canonical name did not.
    pub(crate) alias: Option<&'static str>,
}

/// The rows `query` matches, best match first.
pub(crate) fn entries(query: &str) -> Vec<Entry> {
    entries_in(catalog::CATALOGUE, query)
}

/// The rows `query` matches in `table`, best first. The order is the score's,
/// and equal scores keep the catalogue's order, so the palette's listing does
/// not shuffle between two keystrokes.
pub(crate) fn entries_in(table: &[CommandSpec], query: &str) -> Vec<Entry> {
    let mut scored: Vec<(i32, Entry)> = table
        .iter()
        .filter_map(|spec| {
            let (usage, _rank) = spec.listed?;
            if !spec.surfaces.covers(Surface::Tui) {
                return None;
            }
            let (score, alias) = best(spec, query)?;
            Some((
                score,
                Entry {
                    name: spec.name,
                    usage,
                    summary: spec.description,
                    category: spec.category,
                    alias,
                },
            ))
        })
        .collect();
    scored.sort_by_key(|(score, _)| Reverse(*score));
    scored.into_iter().map(|(_, entry)| entry).collect()
}

/// The best score any spelling of `spec` earns for `query`, and the alias that
/// earned it when the canonical name did not.
fn best(spec: &CommandSpec, query: &str) -> Option<(i32, Option<&'static str>)> {
    let mut best = fuzzy_score(spec.name, query).map(|score| (score, None));
    for candidate in spec.aliases {
        if let Some(score) = fuzzy_score(candidate.name, query)
            && best.is_none_or(|(found, _)| score > found)
        {
            best = Some((score, Some(candidate.name)));
        }
    }
    best
}
