//! The palette's pane, read as the rows a terminal would paint.
//!
//! `02` §11: the list, the selected row's usage, and a box that never outgrows
//! the cells it is given. The list is the catalogue's — never a copy — so the
//! first test reads the expected rows back out of `slash/catalog` and compares.

use super::palette::{NO_MATCH, Palette, USAGE};
use super::palette_fixture::{pane, querying};
use super::palette_rows;
use crate::front::tui::theme::{Charset, Glyph, glyph};
use crate::interactive::tui::wrap::cell_width;
use crate::slash::catalog::{self, Surface};

/// The empty query is the whole palette: every row the `/help` listing carries,
/// is not hidden, and the TUI can run — the everyday commands first and the job
/// verbs last.
#[test]
fn an_empty_query_lists_every_listed_tui_row_everyday_first() {
    let palette = Palette::default();
    let mut expected: Vec<&str> = catalog::all()
        .filter(|spec| spec.listed.is_some() && !spec.hidden)
        .filter(|spec| spec.surfaces.covers(Surface::Tui))
        .map(|spec| spec.name)
        .collect();
    expected.sort_unstable();

    let mut offered: Vec<&str> = palette.entries().iter().map(|entry| entry.name).collect();
    offered.sort_unstable();
    assert_eq!(
        offered, expected,
        "every listed, visible, TUI row is offered exactly once"
    );

    let first: Vec<&str> = palette
        .entries()
        .iter()
        .take(4)
        .map(|entry| entry.name)
        .collect();
    assert_eq!(
        first,
        ["status", "model", "clear", "compact"],
        "the everyday commands open the palette"
    );
    let rows = pane(&palette, 100, 60);
    let listed: Vec<&String> = rows[1..rows.len() - 2]
        .iter()
        .filter(|row| !row.is_empty())
        .collect();
    assert_eq!(
        listed.len(),
        expected.len(),
        "every offered row gets a row of the pane"
    );
    let marked = format!("{} /", glyph(Glyph::Selected, Charset::Utf8));
    assert!(
        listed[0].starts_with(&marked),
        "the best match is marked: {:?}",
        listed[0]
    );
    assert!(
        listed
            .iter()
            .all(|row| row.starts_with(&marked) || row.starts_with("  /")),
        "each row names its command: {:?}",
        &listed[..3]
    );
}

/// A query nothing matches says so, and offers nothing.
#[test]
fn a_query_that_matches_nothing_says_so() {
    let palette = querying("zzzz");
    assert!(
        palette.entries().is_empty(),
        "zzzz matches no catalogued name"
    );
    assert!(palette.selected_entry().is_none());

    let rows = pane(&palette, 100, 24);
    assert!(
        rows.iter()
            .any(|row| row.contains(&format!("{NO_MATCH} \"zzzz\""))),
        "the empty row names the query: {rows:?}"
    );
    assert!(
        !rows.iter().any(|row| row.contains(" /")),
        "no command row is drawn: {rows:?}"
    );
    assert!(
        !rows.iter().any(|row| row.contains(USAGE)),
        "no usage row without a selection: {rows:?}"
    );
}

/// The usage row is the catalogue's own synopsis for the selected row.
#[test]
fn the_usage_row_is_the_catalogues_synopsis() {
    let palette = querying("exp");
    let entry = palette.selected_entry().expect("exp matches a command");
    assert_eq!(entry.name, "export", "the catalogue's order breaks the tie");

    let spec = catalog::all()
        .find(|spec| spec.name == entry.name)
        .expect("a palette row is a catalogue row");
    let (synopsis, _rank) = spec.listed.expect("a palette row is a listed row");
    assert_eq!(entry.usage, synopsis, "the synopsis is the catalogue's");
    assert_eq!(
        entry.summary, spec.description,
        "so is the one-line summary"
    );

    let rows = pane(&palette, 100, 24);
    let usage = rows
        .iter()
        .find(|row| row.starts_with(USAGE))
        .expect("the pane carries a usage row");
    assert_eq!(
        usage,
        &format!("{USAGE} {synopsis}"),
        "the usage row is the synopsis, whole"
    );
}

/// The selection moves by one and stops at both ends of the list.
#[test]
fn the_selection_stops_at_both_ends() {
    let mut palette = querying("exp");
    assert_eq!(palette.entries().len(), 2, "exp matches two commands");

    palette.down();
    assert_eq!(palette.selected(), 1);
    palette.down();
    palette.down();
    assert_eq!(palette.selected(), 1, "the last row is the last row");

    palette.up();
    assert_eq!(palette.selected(), 0);
    palette.up();
    palette.up();
    assert_eq!(palette.selected(), 0, "the first row is the first row");

    let mut nothing = querying("zzzz");
    nothing.down();
    nothing.up();
    assert_eq!(nothing.selected(), 0, "an empty list selects nothing");
}

/// The selected row stays on screen: a pane shorter than the list scrolls to
/// it rather than hiding it.
#[test]
fn the_selected_row_stays_on_screen_in_a_short_pane() {
    let mut palette = Palette::default();
    for _ in 0..10 {
        palette.down();
    }
    let selected = palette.entries()[palette.selected()].name;
    assert_eq!(palette.selected(), 10);

    let rows = pane(&palette, 100, 8);
    assert!(
        rows.iter()
            .any(|row| row.contains(&format!("/{selected} "))),
        "the eleventh row is on screen: {rows:?}"
    );
    assert_eq!(rows.len(), 8, "the pane is the height it was given");
}

/// No row of any pane outgrows the cells it was given, at any width the pane
/// can be asked for and for any query.
#[test]
fn no_rendered_row_is_wider_than_the_pane() {
    for query in ["", "e", "aprv", "zzzz"] {
        let palette = querying(query);
        for (width, height) in [(100, 30), (80, 24), (60, 12), (24, 8)] {
            let rows = palette_rows::rows(&palette, width, height);
            assert_eq!(
                rows.len(),
                height,
                "{query:?} at {width}x{height} is a full pane"
            );
            for row in &rows {
                assert!(
                    cell_width(&row.text()) <= width,
                    "{:?} is wider than {width} at {query:?}: {:?}",
                    row.text(),
                    cell_width(&row.text())
                );
            }
        }
    }
}

/// `--ascii` reaches the pane: the same words, in the seven-bit glyph set.
#[test]
fn the_ascii_pane_says_the_same_things() {
    let palette = Palette {
        charset: Charset::Ascii,
        ..querying("exp")
    };
    let rows = pane(&palette, 100, 24);

    assert!(rows[0].starts_with("Commands"), "{:?}", rows[0]);
    assert!(
        rows[0].contains(" > exp"),
        "the query keeps its mark: {:?}",
        rows[0]
    );
    assert!(
        rows[1].starts_with("> /export"),
        "the selection mark falls back: {:?}",
        rows[1]
    );
    let last = rows.last().expect("the pane has a bottom border");
    assert!(
        last.starts_with("[enter] insert - Tab complete - Esc close"),
        "the key row falls back whole: {last:?}"
    );
}
