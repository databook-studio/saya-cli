//! `saya job list` rendering: one row per job, the origin named in its own
//! column so a listing says where each job was opened from.

use super::*;

fn command(all: bool) -> CodingCommand {
    CodingCommand::List { after: None, all }
}

fn page() -> serde_json::Value {
    json!({
        "jobs":[
            {"job_id":"job-cli","state":"Running","origin":"cli"},
            {"job_id":"job-session","state":"Succeeded","origin":"session"}
        ],
        "next_cursor":null
    })
}

/// The row of one job, as the listing prints it.
fn row<'a>(text: &'a str, job: &str) -> &'a str {
    text.lines()
        .find(|line| line.split_whitespace().next() == Some(job))
        .unwrap_or_else(|| panic!("no row for {job}: {text}"))
}

#[test]
fn the_origin_is_a_column_of_its_own() {
    let text = render_coding_result(&command(true), &page());
    assert!(
        text.lines()
            .any(|line| line.split_whitespace().last() == Some("origin")),
        "the header names the origin column: {text}"
    );
    assert_eq!(
        row(&text, "job-cli").split_whitespace().last(),
        Some("cli"),
        "the CLI's job shows `cli` in the origin column: {text}"
    );
    assert_eq!(
        row(&text, "job-session").split_whitespace().last(),
        Some("session"),
        "the session's job shows `session` in the origin column: {text}"
    );
    assert!(
        row(&text, "job-session").contains("Succeeded"),
        "the state keeps its column: {text}"
    );
}

#[test]
fn a_default_listing_says_how_to_see_the_hidden_ones() {
    let text = render_coding_result(&command(false), &json!({"jobs":[],"next_cursor":null}));
    assert!(text.contains("no jobs"), "{text}");
    assert!(
        text.contains("--all"),
        "a default listing names the flag that adds session jobs: {text}"
    );
    let text = render_coding_result(&command(true), &json!({"jobs":[],"next_cursor":null}));
    assert!(
        !text.contains("--all"),
        "`--all` hides nothing, so it asks for nothing more: {text}"
    );
}

#[test]
fn a_more_page_is_named_rather_than_dropped() {
    let text = render_coding_result(
        &command(false),
        &json!({
            "jobs":[{"job_id":"job-cli","state":"Running","origin":"cli"}],
            "next_cursor":"job-cli"
        }),
    );
    assert!(
        text.contains("list again after the last id"),
        "a bounded page says how to read on: {text}"
    );
}

#[test]
fn an_unexpected_listing_shape_stays_json() {
    let value = json!({"unexpected":true});
    assert_eq!(
        render_coding_result(&command(true), &value),
        value.to_string()
    );
}
