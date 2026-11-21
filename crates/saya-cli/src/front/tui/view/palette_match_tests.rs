//! The palette's list: which catalogue rows a query offers, in what order, and
//! through which spelling.
//!
//! Every row comes from `slash/catalog`'s lookup, so these tests read the
//! catalogue back rather than restating it — a name added there, or a row the
//! listing folds away, changes the palette without a second list to edit.

use super::palette_fixture::{folded, pane, querying};
use super::palette_match;
use crate::slash::catalog::{self, Category};

/// A row the listing folds under another command is not a palette row.
#[test]
fn a_row_the_help_listing_folds_away_is_never_offered() {
    let table = folded();
    let entries = palette_match::entries_in(&table, "");

    assert_eq!(
        entries.len(),
        1,
        "one offered row, not two: {:?}",
        entries.iter().map(|entry| entry.name).collect::<Vec<_>>()
    );
    assert_eq!(entries[0].name, "offered");
    assert_eq!(entries[0].usage, "/offered <path>");
}

/// An alias spelling ranks its canonical command, and the row says which
/// spelling found it: `aprv` reaches `/mode` through `/approvals`.
#[test]
fn a_typo_of_an_alias_finds_the_canonical_command() {
    let palette = querying("aprv");
    let entries = palette.entries();
    let found = entries
        .iter()
        .find(|entry| entry.alias == Some("approvals"))
        .unwrap_or_else(|| panic!("aprv must reach the approvals alias: {entries:?}"));

    assert_eq!(found.name, "mode", "an alias ranks its canonical command");
    assert_eq!(
        catalog::resolve("approvals"),
        Some("mode"),
        "the catalogue is the one lookup"
    );
    assert_eq!(found.category, Category::Model);

    let rows = pane(&palette, 100, 24);
    assert!(
        rows.iter()
            .any(|row| row.contains("/mode") && row.contains("/approvals")),
        "the row shows the canonical name and notes the alias: {rows:?}"
    );
}

/// A canonical spelling ranks its own row and notes no alias: `/mode` typed as
/// `/mode` is not reached through `/approvals`.
#[test]
fn a_canonical_name_ranks_its_own_row_and_notes_no_alias() {
    let entries = querying("mode").entries();
    let mode: Vec<&_> = entries
        .iter()
        .filter(|entry| entry.name == "mode")
        .collect();

    assert_eq!(
        mode.len(),
        1,
        "one row per command, however many spellings matched"
    );
    assert_eq!(
        mode[0].alias, None,
        "the canonical spelling is not an alias"
    );
}

/// The guard: every row the palette offers is a name the catalogue's lookup
/// answers with that same canonical name.
#[test]
fn every_palette_entry_resolves_through_the_catalogue() {
    for query in ["", "e", "aprv", "co", "job"] {
        let entries = querying(query).entries();
        assert!(!entries.is_empty(), "the query {query:?} matches something");
        for entry in &entries {
            assert_eq!(
                catalog::resolve(entry.name),
                Some(entry.name),
                "{:?} is a catalogued canonical name",
                entry.name
            );
            assert!(
                catalog::names().any(|name| name == entry.name),
                "{:?} is a name the parser accepts",
                entry.name
            );
        }
    }
}
