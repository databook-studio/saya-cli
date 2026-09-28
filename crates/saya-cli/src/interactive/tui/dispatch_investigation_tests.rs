//! Tests for the TUI investigation adapter: the save-without-SQL fill from
//! the session's last selectable query, with the spec's exact refusals.

use super::fill_save_from_last_query;
use crate::cli::InvestigationCommand;
use crate::interactive::tui::types::LastQuery;

fn save(name: &str, sql: Option<&str>, connection: Option<&str>) -> InvestigationCommand {
    InvestigationCommand::Save {
        name: name.into(),
        description: None,
        sql: sql.map(Into::into),
        file: None,
        connection: connection.map(Into::into),
    }
}

fn last_query(sql: &str, connection: Option<&str>) -> Option<LastQuery> {
    Some(LastQuery {
        sql: sql.into(),
        connection: connection.map(Into::into),
    })
}

/// `/investigation save <name>` with no SQL fills sql and connection from
/// the last successful, concrete query — the connection that actually ran it.
#[test]
fn tui_save_uses_latest_successful_query_and_its_connection() {
    let filled = fill_save_from_last_query(
        save("recent", None, None),
        &last_query("SELECT count(*) FROM orders", Some("staging")),
    )
    .unwrap();
    assert_eq!(
        filled,
        save(
            "recent",
            Some("SELECT count(*) FROM orders"),
            Some("staging")
        ),
        "the saved SQL and connection must come from the last query"
    );
}

/// No last query at all: refuse with the spec's message — never save empty.
#[test]
fn tui_save_refuses_without_a_query() {
    let error = fill_save_from_last_query(save("recent", None, None), &None)
        .expect_err("a save with nothing to save must refuse");
    assert!(
        error.contains("Nothing to save yet: run a query first (or pass --sql)."),
        "the refusal names the remedy: {error}"
    );
}

/// The last query ran without naming its connection: refuse — the current
/// profile must never be substituted silently.
#[test]
fn tui_save_refuses_unnamed_connection() {
    let error =
        fill_save_from_last_query(save("recent", None, None), &last_query("SELECT 1", None))
            .expect_err("an unnamed connection must refuse");
    assert!(
        error.contains(
            "The last query did not name its connection; re-run it with /sql, or pass --connection."
        ),
        "the refusal names both remedies: {error}"
    );
}

/// An explicit `--sql` (or `--file`) always wins: the last query is left out
/// entirely — its SQL and its connection alike.
#[test]
fn explicit_sql_wins_over_last_query() {
    let last = last_query("SELECT 1", Some("staging"));
    let kept = fill_save_from_last_query(save("recent", Some("SELECT 2"), None), &last).unwrap();
    assert_eq!(
        kept,
        save("recent", Some("SELECT 2"), None),
        "explicit SQL wins; the last query's connection is not borrowed"
    );
    // An explicit connection with an explicit SQL stays as parsed.
    let both =
        fill_save_from_last_query(save("recent", Some("SELECT 2"), Some("prod")), &last).unwrap();
    assert_eq!(both, save("recent", Some("SELECT 2"), Some("prod")));
}

/// A `--file` save also never fills: the file's SQL is the explicit input.
#[test]
fn explicit_file_wins_over_last_query() {
    let command = InvestigationCommand::Save {
        name: "recent".into(),
        description: None,
        sql: None,
        file: Some(std::path::PathBuf::from("q.sql")),
        connection: None,
    };
    let last = last_query("SELECT 1", Some("staging"));
    assert_eq!(
        fill_save_from_last_query(command.clone(), &last).unwrap(),
        command,
        "a --file save is left as parsed"
    );
}

/// Non-save commands pass through untouched — the fill is save-only.
#[test]
fn non_save_commands_pass_through_untouched() {
    let list = InvestigationCommand::List {
        limit: None,
        offset: None,
    };
    let last = last_query("SELECT 1", Some("staging"));
    assert_eq!(
        fill_save_from_last_query(list.clone(), &last).unwrap(),
        list
    );
    assert_eq!(
        fill_save_from_last_query(list.clone(), &None).unwrap(),
        list
    );
}
