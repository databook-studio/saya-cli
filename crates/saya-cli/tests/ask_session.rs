//! `saya ask` runs as one chat session's one turn.
//!
//! Five rows, each read off a real `saya ask` child against a scripted
//! provider, pin the exit-code table of `docs/commands.md` through the new
//! path: 0 answered, 6 for the paused class (a budget stop and a settled
//! clarification), and 130 for the operator's interrupt. The first row also
//! pins the composition itself: the ask opened a job, which only the chat
//! session's run does.

#[path = "ask_session/harness.rs"]
mod harness;

use std::time::Duration;

use harness::{
    answer_body, clarify_body, fixture, held, reply, run_ask, serve, tool_body, write_config,
};

/// The prose every scripted answer speaks.
const ANSWER: &str = "the-ask-answer";

/// The partial sentence a settled budget stop states, in the shared wording.
const PARTIAL: &str =
    "the run stopped on its elapsed-time budget; any answer it produced is partial or absent";

/// The config every row states: sharing is the operator's consent (the mock is
/// not a local endpoint) and memory is off, so no extraction call spends a
/// scripted reply.
fn config(limits: &str) -> String {
    format!("[ai]\nallow_data_sharing = true\n\n[memory]\nmode = 'off'\n\n{limits}")
}

#[test]
fn a_question_answers_on_a_job_the_ask_opened() {
    let fixture = fixture("answered");
    write_config(&fixture, &config(""));
    let provider = serve(vec![reply(answer_body(ANSWER))]);

    let run = run_ask(
        &fixture,
        provider.address(),
        "how many orders are there",
        Duration::from_secs(60),
    );

    assert!(!run.killed, "the child exited on its own");
    let bodies = provider.bodies();
    assert_eq!(
        bodies.len(),
        1,
        "the mock served one answering round: {bodies:?}"
    );
    assert!(
        bodies[0].contains("how many orders are there"),
        "the answering request carries the question: {}",
        bodies[0]
    );
    assert_eq!(
        run.code,
        Some(0),
        "an answered question exits 0: stdout={} stderr={}",
        run.stdout,
        run.stderr
    );
    assert!(
        run.stdout.contains(ANSWER),
        "the answer reaches the terminal: {}",
        run.stdout
    );
    let jobs = harness::jobs(&fixture);
    assert_eq!(
        jobs.len(),
        1,
        "the ask ran as one chat session: one job under the coding root, got {jobs:?}"
    );
    let _ = std::fs::remove_dir_all(&fixture.root);
}

#[test]
fn a_database_budget_stop_exits_six_with_the_partial_sentence() {
    let fixture = fixture("database-budget");
    write_config(
        &fixture,
        &config("[limits.database]\nelapsed_seconds = 1\n"),
    );
    // The first reply arms the budget by using the database; the second is held
    // past the ceiling it armed, so the turn's own deadline ends it.
    let provider = serve(vec![
        reply(tool_body("schema_discovery", serde_json::json!({}))),
        held(answer_body(ANSWER), Duration::from_secs(15)),
    ]);

    let run = run_ask(
        &fixture,
        provider.address(),
        "how many orders are there",
        Duration::from_secs(90),
    );

    assert!(!run.killed, "the child exited on its own");
    assert_eq!(
        run.code,
        Some(6),
        "a database-budget stop exits 6: stdout={} stderr={}",
        run.stdout,
        run.stderr
    );
    assert_eq!(
        run.stderr
            .lines()
            .filter(|line| line.contains(PARTIAL))
            .count(),
        1,
        "the partial sentence is said once: {}",
        run.stderr
    );
    let _ = std::fs::remove_dir_all(&fixture.root);
}

#[test]
fn an_ask_override_bounds_the_question_and_says_what_to_write_instead() {
    let fixture = fixture("ask-override");
    write_config(&fixture, &config("[limits.ask]\nelapsed_seconds = 1\n"));
    let provider = serve(vec![
        reply(tool_body("schema_discovery", serde_json::json!({}))),
        held(answer_body(ANSWER), Duration::from_secs(15)),
    ]);

    let run = run_ask(
        &fixture,
        provider.address(),
        "how many orders are there",
        Duration::from_secs(90),
    );

    assert!(!run.killed, "the child exited on its own");
    assert_eq!(
        run.code,
        Some(6),
        "the ask override still bounds the question: stdout={} stderr={}",
        run.stdout,
        run.stderr
    );
    assert!(
        run.stderr.contains("[limits.ask] is now [limits.database]"),
        "the retired table says what to write instead: {}",
        run.stderr
    );
    assert_eq!(
        run.stderr
            .lines()
            .filter(|line| line.contains(PARTIAL))
            .count(),
        1,
        "the partial sentence is said once: {}",
        run.stderr
    );
    let _ = std::fs::remove_dir_all(&fixture.root);
}

#[test]
fn a_clarification_exits_six() {
    let fixture = fixture("clarification");
    write_config(&fixture, &config(""));
    let provider = serve(vec![reply(clarify_body("Which one?", ["a", "b"]))]);

    let run = run_ask(
        &fixture,
        provider.address(),
        "which orders do you mean",
        Duration::from_secs(60),
    );

    assert!(!run.killed, "the child exited on its own");
    assert_eq!(
        run.code,
        Some(6),
        "a settled clarification is the paused class: stdout={} stderr={}",
        run.stdout,
        run.stderr
    );
    assert!(
        run.stdout.contains("Which one?"),
        "the question block reaches the terminal: {}",
        run.stdout
    );
    let _ = std::fs::remove_dir_all(&fixture.root);
}

// The row sends the operator's Ctrl-C, a POSIX signal the harness can only
// deliver on Unix.
#[cfg(unix)]
#[test]
fn an_interrupted_question_exits_130() {
    let fixture = fixture("interrupted");
    write_config(&fixture, &config(""));
    let provider = serve(vec![held(answer_body(ANSWER), Duration::from_secs(30))]);

    let run = harness::run_ask_interrupted(
        &fixture,
        provider.address(),
        "how many orders are there",
        Duration::from_secs(2),
        Duration::from_secs(60),
    );

    assert!(!run.killed, "the child exited on its own after the signal");
    assert_eq!(
        run.code,
        Some(130),
        "an interrupted question is the cancelled class: stdout={} stderr={}",
        run.stdout,
        run.stderr
    );
    let _ = std::fs::remove_dir_all(&fixture.root);
}
