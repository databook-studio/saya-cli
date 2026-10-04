//! The ask boundary for a stopped candidate vote: two real attempts nominate
//! the same slow recursive SELECT through `designate_answer`, the configured
//! one-second elapsed ceiling latches a budget stop mid-vote, and the real
//! `saya ask` process must exit 6 with the truthful elapsed-time wording while
//! the earlier answers stay printed — the mock provider sees exactly the two
//! answering requests and nothing after the stop.

#[path = "candidate_vote_stop/harness.rs"]
mod harness;

use std::time::Duration;

use harness::{MockProvider, build_demo, run_ask_bounded, spawn_mock};

/// The canonical slow source (same shape as the traced connector tests).
const SLOW: &str = "WITH RECURSIVE cnt(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM cnt \
        WHERE x < 500000000) SELECT count(*) FROM cnt";

const FIRST_PROSE: &str = "first candidate settles on the recursive count";
const SECOND_PROSE: &str = "second candidate settles on the same recursive count";
const PROMPT: &str = "how many orders are there";

/// One SSE response that both speaks the attempt's prose and designates the
/// answering SQL: the designation comes from the typed `designate_answer`
/// call, not from prose.
fn designation_body(text: &str) -> String {
    let chunk = serde_json::json!({
        "choices": [{
            "delta": {
                "content": text,
                "tool_calls": [{
                    "index": 0,
                    "id": "call-designate",
                    "function": {
                        "name": "designate_answer",
                        "arguments": serde_json::to_string(
                            &serde_json::json!({ "sql": SLOW }),
                        )
                        .unwrap(),
                    },
                }],
            },
        }],
    });
    // The proper stream terminator, spelled so no single token needs escaping.
    let done = format!("data: [{}]\n\n", "DONE");
    format!("data: {chunk}\n\n{done}")
}

#[test]
fn a_stopped_candidate_vote_makes_ask_exit_6_with_the_partial_answer() {
    let fixture = build_demo("candidate-vote-stop");
    // Both ceilings ride the user config (the trusted layer): two independent
    // candidates and the minimum one-second elapsed ceiling.
    let saya_home = fixture.config_home.join("saya");
    std::fs::create_dir_all(&saya_home).expect("the user config home is created");
    std::fs::write(
        saya_home.join("config.toml"),
        "[run]\ncandidates = 2\n\n[ai]\ninvestigation_max_elapsed_seconds = 1\n",
    )
    .expect("the user config with both ceilings is written");
    let mock: MockProvider = spawn_mock(vec![
        designation_body(FIRST_PROSE),
        designation_body(SECOND_PROSE),
    ]);

    let run = run_ask_bounded(&fixture, mock.address(), PROMPT, Duration::from_secs(60));

    // Setup-refusal guard: both answering responses were consumed before the
    // stop — the mock saw exactly the two expected answering requests, each
    // carrying the question, and nothing after the stop.
    let bodies = mock.bodies();
    assert_eq!(
        bodies.len(),
        2,
        "the mock served exactly the two answering rounds: {bodies:?}"
    );
    for body in &bodies {
        assert!(
            body.contains(PROMPT),
            "each answering request carries the question: {body}"
        );
    }

    assert!(
        !run.killed,
        "the ask child exited on its own within the bound: code={:?}",
        run.code
    );
    assert_eq!(
        run.code,
        Some(6),
        "a stopped vote exits 6: stdout={} stderr={}",
        run.stdout,
        run.stderr
    );
    assert!(
        run.stderr.contains("elapsed-time budget"),
        "the truthful elapsed-time wording reaches the terminal: stderr={}",
        run.stderr
    );
    assert!(
        run.stdout.contains(FIRST_PROSE) && run.stdout.contains(SECOND_PROSE),
        "the earlier answers were printed: stdout={}",
        run.stdout
    );
    let _ = std::fs::remove_dir_all(&fixture.root);
}
