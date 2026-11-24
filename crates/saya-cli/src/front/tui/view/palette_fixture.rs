//! Test support for the palette: a palette holding a query, the pane read back
//! as the words a terminal would show, and a catalogue table that folds a row
//! away so the filter has a row to fold.

use super::composer::Row;
use super::palette::Palette;
use super::palette_rows;
use crate::slash::catalog::{Category, CommandSpec};

/// A palette holding `query`.
pub(super) fn querying(query: &str) -> Palette {
    Palette {
        query: query.to_owned(),
        ..Palette::default()
    }
}

/// The pane's rows at `width` x `height`, as their text.
pub(super) fn pane(palette: &Palette, width: usize, height: usize) -> Vec<String> {
    palette_rows::rows(palette, width, height)
        .iter()
        .map(inside)
        .collect()
}

/// One row's text, without its border cells or the padding before the right
/// edge.
pub(super) fn inside(row: &Row) -> String {
    let text = row.text();
    let inner: String = text.chars().skip(2).collect();
    let kept = inner.chars().count().saturating_sub(2);
    let mut trimmed: String = inner.chars().take(kept).collect();
    trimmed.truncate(trimmed.trim_end().len());
    trimmed
}

/// A catalogue of one offered row and one the `/help` listing folds under
/// another: today's table lists every row it holds, so the filter's evidence
/// has to be a table that does not.
pub(super) fn folded() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            name: "offered",
            aliases: &[],
            description: "a row the listing carries",
            category: Category::Query,
            surfaces: crate::slash::catalog::Surfaces::BOTH,
            listed: Some(("/offered <path>", 1)),
            hidden: false,
        },
        CommandSpec {
            name: "folded",
            aliases: &[],
            description: "a row the listing folds under /offered",
            category: Category::Query,
            surfaces: crate::slash::catalog::Surfaces::BOTH,
            listed: None,
            hidden: false,
        },
    ]
}
