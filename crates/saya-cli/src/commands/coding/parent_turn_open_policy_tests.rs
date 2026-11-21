//! The per-turn inputs a surface states at open bound the FIRST turn
//! (phase-4 unit 4.0, master decision 3).
//!
//! `run_open` runs the run's first turn, so a policy a surface could only state
//! afterwards would never bound it. Given at open, a two-request policy stops a
//! three-request first turn with chat's budget wording, exactly as it stops any
//! later turn.

#![cfg(unix)]

use super::super::TurnInputs;
use super::super::parent_turn_fixture::in_place_writing;
use super::super::parent_turn_provider::{Reply, Scripted};
use saya_agent::{NoopEventSink, TurnPolicy};
use std::time::Duration;

/// Load ceiling, not a product bound: how long one scripted turn may take.
const RUN: Duration = Duration::from_secs(120);

/// The narrow policy the test states: one request more than a single call and
/// its answer, so a three-request turn cannot finish.
fn two_requests() -> TurnPolicy {
    TurnPolicy {
        max_requests: Some(2),
        max_tool_calls: None,
        max_known_tokens: None,
        max_elapsed: None,
    }
}

/// Test 3 — a policy given to `run_open` bounds turn 1.
#[tokio::test]
async fn a_policy_given_at_open_bounds_the_first_turn() {
    let provider = Scripted::start_with_script(vec![
        Reply::Write {
            path: "one.txt",
            content: "one\n",
        },
        Reply::Write {
            path: "two.txt",
            content: "two\n",
        },
        Reply::Text("the third request answered"),
    ]);
    let fixture = in_place_writing("open-policy", &provider.endpoint).await;
    let inputs = TurnInputs {
        policy: Some(two_requests()),
        plugins: None,
    };

    let stopped = tokio::time::timeout(RUN, fixture.job.run_open(&NoopEventSink, inputs))
        .await
        .expect("the first turn must settle");
    assert!(
        stopped.is_ok(),
        "a spent question budget is a partial answer, not a run failure: {stopped:?}"
    );
    let stop = fixture
        .job
        .turn_stop()
        .expect("the turn states what stopped it");
    assert_eq!(
        stop.message(),
        "the run stopped on its logical-requests budget; any answer it produced is partial or absent",
        "the first turn stops with chat's own budget wording"
    );
    assert_eq!(
        provider.bodies().len(),
        2,
        "two requests were admitted and the third never arrived"
    );
}
