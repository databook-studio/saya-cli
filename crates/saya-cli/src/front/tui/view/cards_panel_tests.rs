//! The approval panel's words (UI3): which family a call belongs to, the title
//! the panel puts up, the exact command, path, SQL or URL in a block of its
//! own, where it runs, and the answers offered one per line.
//!
//! The panel states the call and nothing else: what it changes is how loudly
//! and how completely, never what is asked or what an answer grants.

use super::cards::{Family, Kind, approval, approval_waiting, clarification};
use super::cards_fixture::{
    FOLDER, NO_FOLDER, SCREEN_ROWS, long_path_question, plan_question, scope_question,
    shell_question, sql_question, ungranted_question, web_question, write_question,
};

/// The keys a card offers, in the order it offers them.
fn keys(card: &super::cards::Card) -> Vec<&'static str> {
    card.choices().iter().map(|choice| choice.key).collect()
}

/// The whole choice column, key and label together.
fn labels(card: &super::cards::Card) -> String {
    card.choices()
        .iter()
        .map(|choice| format!("[{}] {}", choice.key, choice.label))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn a_shell_call_asks_about_a_command_and_shows_it_whole() {
    let card = approval(&shell_question(), FOLDER, SCREEN_ROWS);

    assert_eq!(card.family(), Family::Command);
    assert_eq!(
        card.title(),
        "Allow this command?",
        "the title says what is being allowed, in one line"
    );
    assert_eq!(
        card.target(),
        ["$ python3 -m unittest"],
        "the command is a block of its own, exactly as the call states it"
    );
    let place = card.place().expect("a command says where it runs").line();
    assert!(place.contains("~/shop"), "the folder is named: {place}");
    assert!(
        place.contains("on your machine, not sandboxed"),
        "a shell asks only on the host lane, and says so: {place}"
    );
    assert_eq!(
        labels(&card),
        "[y] Allow once\n[s] Allow for this session · shell:python3\n[n] Deny\n[Esc] Deny",
        "the answers are one per line, the session grant naming what it covers"
    );
}

#[test]
fn a_write_asks_about_a_file_change_and_shows_the_path_whole() {
    let card = approval(&write_question(), FOLDER, SCREEN_ROWS);

    assert_eq!(card.family(), Family::FileChange);
    assert_eq!(card.title(), "Allow this file change?");
    assert_eq!(card.target(), ["src/pricing.py"]);
    let place = card.place().expect("a write says where it lands").line();
    assert!(place.contains("~/shop"), "the folder is named: {place}");
    assert_eq!(
        labels(&card),
        "[y] Allow once\n[s] Allow for this session · workspace-write\n[n] Deny\n[Esc] Deny"
    );
}

#[test]
fn a_query_asks_about_a_query_and_shows_the_statement_whole() {
    let card = approval(&sql_question(), FOLDER, SCREEN_ROWS);

    assert_eq!(card.family(), Family::Query);
    assert_eq!(card.title(), "Allow this query?");
    assert_eq!(
        card.target(),
        [
            "SELECT COUNT(*) AS orders_since_december",
            "  FROM orders",
            " WHERE order_date >= '2025-12-01'",
        ],
        "the statement is stated line for line, never folded"
    );
    assert_eq!(
        card.place().map(|place| place.line()),
        Some(String::from("on connection demo")),
        "a query says the connection it runs on"
    );
    assert!(
        labels(&card).contains("[s] Allow for this session · sql:demo"),
        "the session answer names the connection grant: {}",
        labels(&card)
    );
}

#[test]
fn a_fetch_asks_about_a_web_request_and_shows_the_url_whole() {
    let card = approval(&web_question(), FOLDER, SCREEN_ROWS);

    assert_eq!(card.family(), Family::Web);
    assert_eq!(card.title(), "Allow this web request?");
    assert_eq!(card.target(), ["GET https://example.invalid/pricing"]);
    let shown = card.place().map(|place| place.line());
    assert_eq!(shown.as_deref(), Some("from this machine"));
    assert!(
        labels(&card).contains("fetch:example.invalid"),
        "the session answer names the destination: {}",
        labels(&card)
    );
}

#[test]
fn a_jobs_consent_keeps_its_own_title_and_states_the_scope_whole() {
    let card = approval(&scope_question(), FOLDER, SCREEN_ROWS);

    assert_eq!(card.family(), Family::Scope);
    assert_eq!(card.title(), "Approve this job?");
    assert_eq!(
        card.target()[0],
        "Approve job pty-job with this exact scope:",
        "the consent's first row is untouched"
    );
    let shown = card.place().map(|place| place.line());
    assert_eq!(shown.as_deref(), Some("in ~/shop"));
    assert_eq!(
        keys(&card),
        ["y", "n", "Esc"],
        "a job's consent offers no session grant, so it offers none"
    );
}

#[test]
fn a_jobs_raw_proposal_sits_behind_v_and_not_in_the_panel_body() {
    let card = approval(&scope_question(), FOLDER, SCREEN_ROWS);
    let room = 90;

    assert!(
        !card.target().iter().any(|row| row.starts_with('{')),
        "the panel's body is the human lines only: {:?}",
        card.target()
    );
    assert!(
        card.target()
            .contains(&String::from("Approve job pty-job with this exact scope:")),
        "and the scope line is still the first row: {:?}",
        card.target()
    );
    let folded = card
        .shown_rows(false, room)
        .iter()
        .map(|row| row.text())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !folded.contains("{\"job_id\""),
        "the raw proposal is not on the folded body: {folded}"
    );

    let full = card
        .shown_rows(true, room)
        .iter()
        .map(|row| row.text())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        full.contains("{\"job_id\":\"pty-job\""),
        "`v` opens the exact bytes the yes covers: {full}"
    );
    assert!(
        card.withheld(room) > 0,
        "and the card counts what it is withholding, so `v` says it has more"
    );
}

#[test]
fn a_session_answer_is_offered_only_when_the_call_named_one() {
    let offered = approval(&shell_question(), FOLDER, SCREEN_ROWS);
    assert_eq!(keys(&offered), ["y", "s", "n", "Esc"]);

    let nothing = approval(&ungranted_question(), FOLDER, SCREEN_ROWS);
    assert_eq!(
        keys(&nothing),
        ["y", "n", "Esc"],
        "a call with no token offers no session answer: {}",
        labels(&nothing)
    );
    assert!(
        !labels(&nothing).contains("this session"),
        "and never promises one it cannot record: {}",
        labels(&nothing)
    );
}

#[test]
fn more_than_one_waiting_is_counted_in_the_title() {
    let one = approval(&shell_question(), FOLDER, SCREEN_ROWS);
    assert_eq!(
        one.title(),
        "Allow this command?",
        "one question needs no count"
    );

    let three = approval_waiting(&shell_question(), FOLDER, SCREEN_ROWS, 3);
    assert_eq!(
        three.title(),
        "Allow this command? · 1 of 3",
        "the title says how many are waiting and which one this is"
    );
}

#[test]
fn a_call_longer_than_the_panel_wraps_and_is_never_cut() {
    let card = approval(&long_path_question(), FOLDER, SCREEN_ROWS);
    let whole = card.target().join("\n");
    let room = 30;
    let rows = card.shown_rows(true, room);
    let texts: Vec<String> = rows.iter().map(|row| row.text().to_owned()).collect();

    assert!(rows.len() > 1, "a long path needs more than one row");
    assert!(
        texts.iter().all(|row| super::cards::cells(row) <= room),
        "no row passes the room it was given: {texts:?}"
    );
    assert_eq!(
        texts.concat(),
        whole,
        "the rows join back into the path the operator is approving"
    );
}

#[test]
fn the_body_the_screen_withholds_is_counted_and_viewable() {
    let question = long_path_question();
    let card = approval(&question, FOLDER, SCREEN_ROWS);

    let folded = card.shown_rows(false, 30);
    assert!(
        folded.len() <= super::cards::BODY_ROWS + 1,
        "80x24 shows at most the card's body rows, and the count after them: {folded:?}"
    );
    assert!(
        card.withheld(30) > 0,
        "and counts what it is withholding, so `v` has something to open"
    );
}

#[test]
fn the_plan_keeps_its_two_answers() {
    let card = approval(&plan_question(), FOLDER, SCREEN_ROWS);

    assert_eq!(card.family(), Family::Plan);
    assert_eq!(card.title(), "Approve this plan?");
    assert_eq!(
        labels(&card),
        "[y] Approve plan\n[n] Keep planning\n[Esc] Keep planning",
        "a plan opens or keeps planning, and has no third answer"
    );
}

#[test]
fn a_clarification_offers_its_field_and_no_choices() {
    let card = clarification("Which revenue do you mean?", SCREEN_ROWS);

    assert_eq!(card.kind(), Kind::Question);
    assert_eq!(card.title(), "Question");
    assert!(
        card.choices().is_empty(),
        "a typed answer is not one of a list"
    );
    assert!(card.field().is_some());
}

#[test]
fn a_call_with_no_bound_folder_names_only_what_it_has() {
    let card = approval(&write_question(), NO_FOLDER, SCREEN_ROWS);

    assert_eq!(
        card.place().map(|place| place.line()),
        Some(String::from("inside the workspace")),
        "an unbound launch names no folder it does not have"
    );
    assert_eq!(card.title(), "Allow this file change?");
}

#[test]
fn a_command_in_the_launch_folder_names_that_folder_and_never_a_dot() {
    // The operator's default working directory is the launch's own folder, and
    // the facts state it as `.` — the place line names the folder instead.
    let question = super::cards_fixture::shell_question();
    let question = crate::chat::Question {
        facts: String::from("in: .\n$ cargo test"),
        ..question
    };
    let card = approval(&question, Some("/home/me/demo"), SCREEN_ROWS);

    assert_eq!(
        card.place().map(|place| place.line()),
        Some(String::from("in demo · on your machine, not sandboxed")),
        "the place line names the folder, never `.`"
    );
}
