//! The welcome screen's suggestions: every example names something the
//! workspace holds, and a session with nothing to read gets neutral words.

use super::empty::{EmptyFacts, rows as empty_rows};
use super::empty_suggestions;
use crate::front::tui::theme::Scheme;

/// The rows of the empty state, as the words a terminal shows.
fn shown(facts: &EmptyFacts) -> Vec<String> {
    empty_rows(facts, 100, 30)
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect()
}

/// The no-database welcome inside a bound project.
fn project() -> EmptyFacts {
    EmptyFacts {
        provider: true,
        workspace: Some(String::from("~/shop")),
        scheme: Scheme::Dark,
        ..EmptyFacts::default()
    }
}

#[test]
fn the_welcome_offers_the_projects_own_names() {
    let facts = EmptyFacts {
        suggestions: vec![
            String::from("explain README.md"),
            String::from("explain src/"),
            String::from("add a test to app.py"),
        ],
        ..project()
    };
    let rows = shown(&facts).join("\n");

    for suggestion in ["README.md", "src/", "app.py"] {
        assert!(
            rows.contains(suggestion),
            "the welcome offers {suggestion:?}: {rows}"
        );
    }
    assert!(
        !rows.contains("tests/test_pricing.py"),
        "a path the project does not hold is never offered: {rows}"
    );
}

#[test]
fn a_session_with_no_workspace_gets_neutral_examples() {
    let facts = EmptyFacts {
        workspace: None,
        ..project()
    };
    let rows = shown(&facts).join("\n");

    for line in shown(&facts) {
        assert!(
            !line.contains(".py") && !line.contains('/'),
            "a neutral example names no path: {line:?}"
        );
    }
    assert!(
        !rows.contains("tests/test_pricing.py"),
        "no invented path on a session that has none: {rows}"
    );
}

#[test]
fn suggestions_name_only_entries_the_project_holds() {
    let entries = vec![
        String::from("README.md"),
        String::from("Cargo.toml"),
        String::from("src/"),
        String::from("app.py"),
    ];
    let suggestions = empty_suggestions::from_entries(&entries);

    assert!(
        !suggestions.is_empty(),
        "a project with entries offers something: {entries:?}"
    );
    assert!(
        suggestions.len() <= empty_suggestions::SHOWN,
        "at most {} examples: {suggestions:?}",
        empty_suggestions::SHOWN
    );
    for suggestion in &suggestions {
        assert!(
            entries.iter().any(|entry| suggestion.contains(entry)),
            "the suggestion {suggestion:?} names one of {entries:?}"
        );
    }
    assert!(
        empty_suggestions::from_entries(&[]).is_empty(),
        "an empty folder has nothing to suggest"
    );
}

#[test]
fn the_neutral_examples_are_not_paths() {
    let neutral = empty_suggestions::neutral();
    assert_eq!(neutral.len(), empty_suggestions::SHOWN);
    for example in &neutral {
        assert!(
            !example.contains('/') && !example.contains('.'),
            "a neutral example names no path: {example:?}"
        );
    }
}
