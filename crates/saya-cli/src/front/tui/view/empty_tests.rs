//! The empty transcript's rules besides its copy: which state the facts
//! select, where the examples come from, and the two laws every row keeps —
//! it fits the pane, and with colour off it says the same words.

use super::empty::{EmptyFacts, State, state};
use super::empty_fixture::{
    a_long_question, ascii, cells, colour_off, connected, first_run, no_database, no_workspace,
    rendered, words,
};
use super::empty_rows;
use crate::interactive::tui::ui::surface::starter_questions;

/// A missing provider outranks everything; a missing workspace outranks a
/// connected database; a connected database outranks an empty one.
#[test]
fn the_state_precedence_follows_02_9() {
    assert_eq!(
        state(&EmptyFacts {
            database: Some(super::empty_fixture::demo()),
            ..EmptyFacts::default()
        }),
        State::NoProvider,
        "a run that cannot answer says so before it says anything else"
    );
    assert_eq!(
        state(&EmptyFacts {
            provider: true,
            database: Some(super::empty_fixture::demo()),
            ..EmptyFacts::default()
        }),
        State::NoWorkspace,
        "a run with no project bound reports that before its database"
    );
    assert_eq!(
        state(&EmptyFacts {
            provider: true,
            ..EmptyFacts::default()
        }),
        State::NoWorkspace,
        "no workspace outranks no database too"
    );
    assert_eq!(
        state(&no_database()),
        State::NoDatabase,
        "ready, in a project, with no database"
    );
    assert_eq!(
        state(&connected()),
        State::Connected,
        "ready, in a project, with a database"
    );
}

/// The connected state's examples are `starter_questions`' first three for the
/// cached schema — read from the function, never copied beside it.
#[test]
fn the_examples_are_the_starter_questions_first_three() {
    let questions = starter_questions(&super::empty_fixture::demo_schema());
    assert_eq!(
        questions.len(),
        3,
        "the fixture's schema asks three questions"
    );
    let rows = rendered(&empty_rows(&connected(), 100, 30));
    for (index, question) in questions.iter().enumerate() {
        assert_eq!(
            rows.get(3 + index),
            Some(&format!("    › {question}")),
            "example {} is the starter question at the same index",
            index + 1
        );
    }
    assert_eq!(rows.len(), 6, "three examples and nothing after them");
}

/// A schema that suggests nothing keeps the intro's invitation and drops the
/// promise of examples: `For example:` with nothing under it would be a lie.
#[test]
fn a_database_without_questions_does_not_say_for_example() {
    let rows = rendered(&empty_rows(
        &EmptyFacts {
            starter_questions: Vec::new(),
            ..connected()
        },
        100,
        30,
    ));
    assert_eq!(
        rows,
        vec![
            "  Saya · ~/shop · demo (sqlite, 12 tables)",
            "",
            "  Ask about demo or describe a change.",
        ]
    );
}

/// No row is wider than the pane, whichever state draws it and however long a
/// starter question is.
#[test]
fn no_row_exceeds_the_width() {
    for width in [100, 80, 60] {
        for facts in [
            first_run(),
            no_database(),
            connected(),
            no_workspace(),
            a_long_question(),
        ] {
            let rows = empty_rows(&facts, width, 30);
            assert!(!rows.is_empty(), "the state always says something");
            for row in &rows {
                let used = cells(row);
                assert!(
                    used <= width,
                    "a row of {used} cells does not fit {width}: {:?}",
                    words(row)
                );
            }
        }
    }
}

/// `02` §12 rule 2: with colour off the screen says the same words.
#[test]
fn colour_off_paints_the_same_words() {
    for facts in [first_run(), no_database(), connected(), no_workspace()] {
        assert_eq!(
            rendered(&empty_rows(&facts, 100, 30)),
            rendered(&empty_rows(&colour_off(&facts), 100, 30)),
            "colour is never the only thing a row says"
        );
    }
}

/// `--ascii` swaps every glyph the empty states draw: the box's corners and
/// rule, its edges, and the examples' marker.
#[test]
fn the_ascii_glyphs_are_the_fallback_ones() {
    let connected = rendered(&empty_rows(&ascii(&connected()), 100, 30));
    assert_eq!(
        connected[3], "    > What are the most common channel values in customer_contacts?",
        "the examples' marker is the ASCII prompt"
    );
    let welcome = rendered(&empty_rows(&ascii(&first_run()), 100, 30));
    assert!(
        welcome[0].trim_end().ends_with('+') && welcome[0].contains('-'),
        "the box's top border is drawn in ASCII: {:?}",
        welcome[0]
    );
    assert!(
        welcome[1].trim_start().starts_with("|  Welcome to Saya"),
        "the box's edge is drawn in ASCII: {:?}",
        welcome[1]
    );
}

/// The pane never answers with more rows than it was given.
#[test]
fn a_short_pane_shows_only_its_rows() {
    assert_eq!(empty_rows(&no_database(), 100, 3).len(), 3);
    assert_eq!(empty_rows(&first_run(), 100, 2).len(), 2);
}
