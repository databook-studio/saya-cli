//! The project's untrusted context rides a chat turn's user lane.
//!
//! The old turn body's projection pins (`agent/context/preferences_tests/
//! runtime_turn.rs`, `agent/context/sources_tests/runtime_turn.rs`) said three
//! things about one assembled turn: the workspace `AGENTS.md` reaches the
//! request as a labelled user-turn block and never the system prompt, the
//! worktree's explicit preferences follow it on the same lane, and the whole
//! projection stays inside the configured context byte budget while leaving a
//! quarter of it for history and the answer.
//!
//! This is that sentence on the live path: a composed chat session's root turn
//! projects through `agent::context::projection::project_untrusted_context`
//! (`commands::coding::parent_loop`) because its plugin context states the
//! `Chat` surface.

#![cfg(unix)]

use std::time::Duration;

use saya_agent::{ApprovalPolicy, NoopEventSink};
use saya_store::ProjectPreferenceStore;

use super::compose::RunComposition;
use super::compose_fixture::{self, RootGuard};
use super::operator_front_fixture;
use crate::commands::coding::parent_turn_provider::{Reply, Scripted};
use crate::commands::preferences::worktree_id_for;

/// Load ceiling, not a product bound: how long one scripted turn may take.
const RUN: Duration = Duration::from_secs(120);

const INSTRUCTION_MARKER: &str = "AGENTS-MARKER-the-project-guidance-line";
const PREFERENCE_MARKER: &str = "PREFERENCE-MARKER-the-worktree-preference-line";
const INSTRUCTION_LABEL: &str = "project-instructions (AGENTS.md)";
const PREFERENCE_LABEL: &str = "project-preferences";

/// The context byte budget the trimmed case states: small enough that the
/// projection's quarter reserve — not the loader's own cap — is what trims the
/// instructions.
const TIGHT_BUDGET: usize = 32 * 1024;

/// The composed run over the fixture, the folder kept alive, and the first
/// request body its first turn sent.
async fn first_request(
    case: &str,
    budget: Option<usize>,
    guidance: &str,
) -> (String, usize, RootGuard) {
    let provider = Scripted::start_with_script(vec![Reply::Text("hello")]);
    let fixture = match budget {
        None => compose_fixture::in_place(case, &provider.endpoint).await,
        Some(budget) => {
            compose_fixture::in_place_with_budget(case, &provider.endpoint, budget).await
        }
    };
    let front = operator_front_fixture::non_interactive();
    let composed = fixture.into_parts(front.front(), case, ApprovalPolicy::Ask);
    // The folder the run opens over: the project instruction source is the
    // `AGENTS.md` at its root, and the preference is stored under its identity.
    let folder = composed.root.checkout_root();
    std::fs::write(folder.join("AGENTS.md"), guidance).expect("the fixture writes its AGENTS.md");
    let worktree = worktree_id_for(&composed.application.root_path())
        .expect("the run's folder derives a worktree id");
    composed
        .application
        .store()
        .add_project_preference(&worktree, PREFERENCE_MARKER, 1)
        .await
        .expect("the preference is stored");
    let composition = RunComposition::compose(composed.application, composed.run)
        .await
        .expect("the session's run composes");
    let budget = composition
        .job()
        .runtime_config()
        .resolved
        .ai
        .context_byte_budget;
    composition.open().await.expect("the run opens");
    let turn = tokio::time::timeout(
        RUN,
        composition
            .job()
            .run_next_turn(&NoopEventSink, "what is in the folder"),
    )
    .await
    .expect("the turn settles");
    assert!(turn.is_ok(), "the turn must run: {turn:?}");
    let body = provider
        .bodies()
        .into_iter()
        .next()
        .expect("the turn sent one request");
    (body, budget, composed.root)
}

/// The messages one provider request carried.
fn messages(body: &str) -> Vec<serde_json::Value> {
    let body: serde_json::Value = serde_json::from_str(body).expect("the provider request is JSON");
    body["messages"]
        .as_array()
        .cloned()
        .expect("the request states its messages")
}

fn text(message: &serde_json::Value) -> String {
    message["content"].as_str().unwrap_or_default().to_owned()
}

#[tokio::test]
async fn instructions_and_preferences_ride_the_chat_turns_user_lane() {
    let (body, _budget, _root) =
        first_request("context-lane", None, &format!("{INSTRUCTION_MARKER}\n")).await;
    let messages = messages(&body);
    let system = &messages[0];
    let user = messages.last().expect("the turn's own prompt is last");
    assert_eq!(
        system["role"], "system",
        "the first message is the system prompt"
    );
    assert_eq!(
        user["role"], "user",
        "the turn's own prompt is the user turn"
    );

    assert!(
        !text(system).contains(INSTRUCTION_MARKER),
        "instruction text must never reach the system message: {system}"
    );
    assert!(
        !text(system).contains(PREFERENCE_MARKER),
        "preference text must never reach the system message: {system}"
    );
    let lane = text(user);
    assert!(
        lane.contains(INSTRUCTION_LABEL) && lane.contains(INSTRUCTION_MARKER),
        "the project instructions ride the user turn, labelled: {lane}"
    );
    assert!(
        lane.contains(PREFERENCE_LABEL) && lane.contains(PREFERENCE_MARKER),
        "the worktree's preferences ride the same user turn: {lane}"
    );
    let instructions_at = lane.find(INSTRUCTION_LABEL).expect("label present");
    let preferences_at = lane.find(PREFERENCE_LABEL).expect("label present");
    assert!(
        instructions_at < preferences_at,
        "preferences follow the instructions on the lane: {lane}"
    );
}

#[tokio::test]
async fn the_projected_request_stays_inside_the_context_byte_budget() {
    // Nearly thirty kilobytes of guidance under a thirty-two kilobyte budget:
    // the projection's quarter reserve is what keeps the request from spending
    // the whole allowance, so the fit trims the instructions rather than
    // admitting them whole.
    let mut guidance = format!("{INSTRUCTION_MARKER}\n");
    guidance.push_str(&"guidance ".repeat(3_072));
    let (body, budget, _root) =
        first_request("context-budget", Some(TIGHT_BUDGET), &guidance).await;
    assert_eq!(budget, TIGHT_BUDGET, "the fixture stated the budget");
    let messages = messages(&body);
    let request_bytes: usize = messages
        .iter()
        .map(|message| message["role"].as_str().unwrap_or_default().len() + text(message).len())
        .sum();
    assert!(
        request_bytes <= budget * 3 / 4,
        "the projection leaves a quarter of the budget for history and the answer: \
         {request_bytes} of {budget}"
    );
    assert!(
        request_bytes <= budget,
        "the request fits the configured budget: {request_bytes} of {budget}"
    );
    let lane = text(messages.last().expect("the turn's own prompt is last"));
    assert!(
        lane.contains("truncat"),
        "the trimmed instructions carry the visible truncation marker: {lane}"
    );
}
