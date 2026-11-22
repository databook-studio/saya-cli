//! The last SQL the run executed rides the NEXT turn's user tail, never the
//! cached prefix.
//!
//! The old body pinned this
//! (`agent/runtime_budget_tests/context_budget.rs::actual_runtime_request_prefix_is_stable_and_last_sql_stays_in_the_user_tail`):
//! "fixed runtime inputs must produce the same complete request on all five
//! runs", with the last-SQL hint carried on the user turn — a hint that changed
//! per turn would forfeit the provider's prefix cache if it sat in the system
//! prompt.
//!
//! On the live path the hint is the database plugin's
//! (`plugins/database/mod.rs::context` → `last_sql_hint_block`), and it rode no
//! live turn at all: nothing called `record_last_sql`, so `context()` always
//! found `None`. This is the same two sentences on a composed chat run — turn
//! one queries, turn two carries the query on its user lane, and the system
//! prompt does not move a byte between them.

use std::time::Duration;

use saya_agent::ApprovalPolicy;
use serde_json::json;

use super::compose::RunComposition;
use super::compose_fixture;
use super::operator_front_fixture;
use crate::commands::coding::parent_turn_provider::{Reply, Scripted};

/// Load ceiling, not a product bound: how long one scripted turn may take.
const RUN: Duration = Duration::from_secs(120);

/// The SQL turn one runs and turn two should be told about.
const SQL: &str = "SELECT name FROM users ORDER BY id ASC LIMIT 1";

/// The hint's own opening words, as the shared block builder words them.
const HINT: &str = "the most recent SQL you ran was";

/// The `system` messages of one request body, in order.
fn system_messages(body: &str) -> Vec<String> {
    let value: serde_json::Value = serde_json::from_str(body).expect("the request body is JSON");
    value["messages"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter(|message| message["role"] == "system")
        .filter_map(|message| message["content"].as_str().map(str::to_owned))
        .collect()
}

/// Whether one request body carries `needle` on a message of `role`.
fn carries(body: &str, role: &str, needle: &str) -> bool {
    let value: serde_json::Value = serde_json::from_str(body).expect("the request body is JSON");
    value["messages"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter(|message| message["role"] == role)
        .filter_map(|message| message["content"].as_str())
        .any(|content| content.contains(needle))
}

#[tokio::test]
async fn the_last_sql_rides_the_next_turns_user_tail_and_the_prefix_stays() {
    // Turn one queries and answers; turn two is answered from what it was told.
    let provider = Scripted::start_with_script(vec![
        Reply::Call {
            name: "bounded_sql_query",
            arguments: json!({ "sql": SQL }),
        },
        Reply::Text("The first user is ada."),
        Reply::Text("The same query still answers it."),
    ]);
    let fixture = compose_fixture::in_place_with_database("last-sql", &provider.endpoint).await;
    let front = operator_front_fixture::non_interactive();
    let composed = fixture.into_parts(front.front(), "session-last-sql", ApprovalPolicy::ReadOnly);
    let composition = RunComposition::compose(composed.application, composed.run)
        .await
        .expect("the session's run composes");
    let _root = composed.root;
    composition.open().await.expect("the run opens");
    let sink = super::learning_provider_fixture::Recording::default();

    let first = tokio::time::timeout(
        RUN,
        composition.job().run_next_turn(&sink, "who are the users?"),
    )
    .await
    .expect("the first turn settles")
    .expect("the first turn answers");
    assert!(
        first.answer.contains("ada"),
        "the first turn ran the query and answered from it: {first:?}"
    );
    let second = tokio::time::timeout(
        RUN,
        composition
            .job()
            .run_next_turn(&sink, "and the same one again?"),
    )
    .await
    .expect("the second turn settles")
    .expect("the second turn answers");
    assert!(
        second.answer.contains("still answers"),
        "the second turn answered: {second:?}"
    );

    let bodies = provider.bodies();
    assert_eq!(
        bodies.len(),
        3,
        "turn one's two requests and turn two's: {bodies:?}"
    );
    // The hint is per turn and never in the system prompt: a request that
    // carried it there would move the bytes a provider caches.
    for body in &bodies {
        assert!(
            !carries(body, "system", HINT),
            "the hint must never reach the system prompt: {body}"
        );
    }
    assert!(
        !carries(&bodies[0], "user", HINT),
        "the first turn has no earlier SQL to be told about: {}",
        bodies[0]
    );
    assert!(
        carries(&bodies[2], "user", SQL),
        "the last SQL stays in the user tail of the next turn: {}",
        bodies[2]
    );
    assert!(
        carries(&bodies[2], "user", HINT),
        "the next turn reads the hint, not the bare statement: {}",
        bodies[2]
    );
    assert_eq!(
        system_messages(&bodies[0]),
        system_messages(&bodies[2]),
        "the request prefix is stable across the two turns"
    );
}
