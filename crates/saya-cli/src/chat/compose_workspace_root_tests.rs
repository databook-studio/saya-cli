//! The chat session's system prompt states the workspace root it works in.
//!
//! The old session stated it in its `Session facts` section
//! (`agent/session_facts.rs::session_facts_text`, "- Workspace root: …."), and
//! `agent/system_prompt_tests/session_facts.rs` pinned it: "a workspace session
//! has facts", and one section per session, never per turn.
//!
//! On the live path the root's prompt is composed once at open
//! (`commands/coding/parent_setup.rs`) from the core's prompt and the active
//! plugins' sections, and no section named the folder the run works in. This is
//! the same fact, on the live chat turn: stated once in the root composition, so
//! every turn of the run sends the same bytes and the provider's prefix cache
//! still holds.

use std::time::Duration;

use saya_agent::ApprovalPolicy;

use super::compose::RunComposition;
use super::compose_fixture;
use super::operator_front_fixture;
use crate::commands::coding::parent_turn_provider::{Reply, Scripted};

/// Load ceiling, not a product bound: how long one scripted turn may take.
const RUN: Duration = Duration::from_secs(120);

/// The fact's own wording, as the shared section builder words it.
const FACT: &str = "- Workspace root:";

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

#[tokio::test]
async fn the_workspace_root_rides_the_system_prompt_once_and_stays_put() {
    let provider = Scripted::start_with_script(vec![
        Reply::Text("the first answer"),
        Reply::Text("the second"),
    ]);
    let fixture =
        compose_fixture::in_place_with_database("workspace-root", &provider.endpoint).await;
    let front = operator_front_fixture::non_interactive();
    let composed = fixture.into_parts(
        front.front(),
        "session-workspace-root",
        ApprovalPolicy::ReadOnly,
    );
    let root = composed.root.checkout_root();
    let composition = RunComposition::compose(composed.application, composed.run)
        .await
        .expect("the session's run composes");
    let _root = composed.root;
    composition.open().await.expect("the run opens");
    let sink = super::learning_provider_fixture::Recording::default();

    for message in ["where do you work?", "and again?"] {
        tokio::time::timeout(RUN, composition.job().run_next_turn(&sink, message))
            .await
            .expect("the turn settles")
            .expect("the turn answers");
    }

    let bodies = provider.bodies();
    assert_eq!(bodies.len(), 2, "one request per turn: {bodies:?}");
    let system = system_messages(&bodies[0]).join("\n");
    assert!(
        system.contains(&format!("{FACT} {}.", root.display())),
        "the session's prompt states the root it works in ({}): {system}",
        root.display()
    );
    assert_eq!(
        system.matches(FACT).count(),
        1,
        "the root is stated once, not once per section: {system}"
    );
    assert_eq!(
        system_messages(&bodies[0]),
        system_messages(&bodies[1]),
        "same session, same bytes: the prefix is stable across turns"
    );
}
