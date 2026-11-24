//! The evidence rule's own table: the words and the one phrase a request may
//! state, and the words that merely look like them.
//!
//! The rule is read here on a run bound to one SQLite connection named `local`
//! and no cached schema, so a request that states a word is answered by the word
//! alone — never by a connection's name or a table the schema happens to hold.

use saya_types::SqlDialect;

use super::evidence::shows_work;
use super::family::Bound;

/// The run every row is read on: one connection, nothing cached.
fn bound() -> Bound {
    Bound::new(
        vec![(String::from("local"), SqlDialect::Sqlite)],
        Vec::new(),
    )
}

/// Whether `text` shows database work on that run.
fn shows(text: &str) -> bool {
    shows_work(text, &bound())
}

/// The counting question: `how many` is the phrase a person writes when they
/// want a number out of a database, in either order the words arrive.
#[test]
fn the_counting_phrase_is_database_work() {
    for request in [
        "how many orders are there",
        "How many did we ship last week?",
        "how many rows in orders",
    ] {
        assert!(shows(request), "{request} asks for a count");
    }
}

/// The two plain words that ask the same, in the plural and verb forms the
/// rule folds: `totals`, `totaled` and `data` itself.
#[test]
fn the_counting_words_are_database_work() {
    for request in [
        "what is the total for the year",
        "show me the totals by month",
        "we totaled the invoices",
        "pull the data behind last week",
    ] {
        assert!(
            shows(request),
            "{request} asks for the database's own figures"
        );
    }
}

/// The rule stays short: a word is not read out of a longer one, so `summary`
/// is not `sum`, `country` is not `count`, `metadata` is not `data`, and a
/// request about none of it latches nothing.
#[test]
fn a_longer_word_is_not_a_counting_word() {
    for request in [
        "why is the monthly summary slow?",
        "chart the country of the census site",
        "tidy the metadata of the README",
        "fix the typo in README",
    ] {
        assert!(!shows(request), "{request} asks for no database");
    }
}
