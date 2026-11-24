//! The palette's order and its words: everyday commands first, jobs last,
//! prefix matches above fuzzy ones, and a description cut where a word ends.

use super::palette::{Palette, USAGE};
use super::palette_fixture::{pane, querying};
use super::palette_frame::fit_words;
use super::palette_match::entries;
use crate::slash::catalog::Category;

/// The names `query` matches, best first.
fn names(query: &str) -> Vec<&'static str> {
    entries(query).iter().map(|entry| entry.name).collect()
}

#[test]
fn the_empty_palette_opens_on_everyday_commands() {
    let listed = names("");
    let first: Vec<&str> = listed.iter().take(4).copied().collect();
    assert_eq!(
        first,
        ["status", "model", "clear", "compact"],
        "everyday commands open the palette: {listed:?}"
    );

    let start = listed.iter().position(|name| *name == "start").unwrap();
    let status = listed.iter().position(|name| *name == "status").unwrap();
    assert!(status < start, "a job starts after the everyday commands");

    let last = entries("");
    let last = last.last().expect("the palette lists commands");
    assert_eq!(
        last.category,
        Category::Jobs,
        "jobs come last: {:?}",
        listed
    );
}

#[test]
fn prefix_matches_rank_above_fuzzy_ones() {
    let listed = names("sta");
    let status = listed.iter().position(|name| *name == "status").unwrap();
    let start = listed.iter().position(|name| *name == "start").unwrap();
    let fuzzy = listed
        .iter()
        .position(|name| *name == "investigation")
        .expect("investigation fuzzy-matches sta");

    assert!(
        status < start,
        "status is the everyday prefix match, so it leads: {listed:?}"
    );
    assert!(
        start < fuzzy,
        "every prefix match outranks a fuzzy one: {listed:?}"
    );
}

#[test]
fn a_cut_description_stops_at_a_word() {
    assert_eq!(
        fit_words("Start a job for long or background work", 14),
        "Start a job…",
        "a cut description keeps whole words"
    );
    assert_eq!(
        fit_words("Save, list, show, run", 8),
        "Save,…",
        "the first word that fits is kept whole"
    );
    assert_eq!(
        fit_words("short", 20),
        "short",
        "a description that fits is not cut"
    );
}

#[test]
fn a_narrow_palette_never_cuts_a_description_mid_word() {
    let palette = querying("sta");
    let rows = pane(&palette, 60, 24);
    let start = rows
        .iter()
        .find(|row| row.contains("/start"))
        .unwrap_or_else(|| panic!("the start row paints: {rows:#?}"));
    let cut = start.split('…').next().expect("a cut row has a prefix");
    let summary = cut
        .split("Start")
        .nth(1)
        .unwrap_or_else(|| panic!("the summary follows the name: {start:?}"));
    let last = summary.split_whitespace().last().unwrap_or_default();
    assert!(
        "Start a job for long or background work; ordinary work is just a message"
            .split_whitespace()
            .any(|word| word == last),
        "the cut stops on a whole word, not at {last:?}: {start:?}"
    );
}

#[test]
fn the_selected_rows_usage_is_named_under_the_list() {
    let palette = Palette::default();
    let rows = pane(&palette, 100, 24);
    assert!(
        rows.iter().any(|row| row.starts_with(USAGE)),
        "the selected row's synopsis rides under the list: {rows:#?}"
    );
}
