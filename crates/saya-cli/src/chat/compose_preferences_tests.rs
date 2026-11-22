//! The slash form and the next request: a forgotten preference is gone.
//!
//! The old body pinned the request half only
//! (`agent/context/preferences_tests/runtime_turn.rs::
//! forget_is_visible_on_the_next_request`): a forget, then "no block after
//! forget". This is that sentence joined to the slash form a chat operator
//! types: `/preferences forget <id>` parses to the chat layer's own read
//! operation (`chat/ops.rs`), that operation is the shared `run_in_session` the
//! line loop and the TUI run, and the *next* turn's request carries no
//! preference block at all — no cache to clear, because the blocks are rebuilt
//! per turn.

use std::sync::Arc;
use std::time::Duration;

use saya_agent::{ApprovalPolicy, NoopEventSink};

use super::compose::RunComposition;
use super::compose_fixture;
use super::operator_front_fixture;
use crate::commands::coding::{CodingApplication, parent_turn_provider::Scripted};
use crate::commands::preferences::operations::add_preference;
use crate::commands::preferences::{PreferenceOutcome, worktree_id_for};
use crate::slash::SlashCommand;

/// Load ceiling, not a product bound: how long one scripted turn may take.
const RUN: Duration = Duration::from_secs(120);

/// The preference text, distinctive so a request can be searched for it.
const TEXT: &str = "prefer tabs over spaces";

/// One composed chat run, the application whose store it reads, the provider
/// whose request bodies it answers, and the folder that outlives it.
async fn composed(
    case: &str,
) -> (
    RunComposition,
    Arc<CodingApplication>,
    Scripted,
    compose_fixture::RootGuard,
) {
    let provider = Scripted::start();
    let fixture = compose_fixture::in_place(case, &provider.endpoint).await;
    let application = Arc::clone(&fixture.application);
    let front = operator_front_fixture::non_interactive();
    let composed = fixture.into_parts(front.front(), case, ApprovalPolicy::ReadOnly);
    let composition = RunComposition::compose(composed.application, composed.run)
        .await
        .expect("the session's run composes");
    let root = composed.root;
    composition.open().await.expect("the run opens");
    (composition, application, provider, root)
}

#[tokio::test]
async fn a_forgotten_preference_is_gone_from_the_next_request() {
    let (composition, application, provider, root) = composed("preferences-forget").await;
    let store = application.store().clone();
    let worktree = worktree_id_for(&root.checkout_root()).expect("the run's folder is a worktree");
    let PreferenceOutcome::Added(record) = add_preference(&store, &worktree, TEXT).await else {
        panic!("the fixture's preference is stored");
    };

    // Turn 1: the block rides the user turn.
    tokio::time::timeout(
        RUN,
        composition.job().run_next_turn(&NoopEventSink, "hello"),
    )
    .await
    .expect("the turn settles")
    .expect("the turn runs");
    let first = provider.bodies().pop().expect("one request");
    assert!(
        first.contains("project-preferences") && first.contains(TEXT),
        "the first turn carries the block: {first}"
    );

    // The slash form, parsed by the one parser every surface uses, and
    // performed by the one shared operation every surface runs.
    let Some(SlashCommand::Preferences(command)) =
        crate::slash::parse_slash_command(&format!("/preferences forget {}", record.id.as_str()))
            .expect("the slash form parses")
    else {
        panic!("`/preferences forget` is the preference command");
    };
    let (code, message) =
        crate::commands::preferences::run_in_session(command, Some(&root.checkout_root()), &store)
            .await;
    assert_eq!(
        code, 0,
        "the slash form's own operation succeeds: {message}"
    );
    assert!(
        message.contains(record.id.as_str()),
        "the forget names the id it removed: {message}"
    );

    // Turn 2: the very next request rebuilds its blocks, so the block is gone.
    tokio::time::timeout(
        RUN,
        composition.job().run_next_turn(&NoopEventSink, "again"),
    )
    .await
    .expect("the turn settles")
    .expect("the turn runs");
    let second = provider.bodies().pop().expect("a second request");
    assert!(
        !second.contains("project-preferences"),
        "no block after forget: {second}"
    );
    assert!(
        !second.contains(TEXT),
        "no preference text after forget: {second}"
    );
}
