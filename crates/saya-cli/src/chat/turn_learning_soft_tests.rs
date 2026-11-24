//! A failed or silent extraction never fails the turn, and is said out loud.
//!
//! Three of the old body's post-turn pins said this of one answered turn:
//! `an_extraction_with_no_response_emits_no_usage_event` ("no response means no
//! usage event"), `test_runtime_extraction_failure_never_fails_turn` ("turn
//! completes despite extraction failure (fail-soft)"), and
//! `a_turn_whose_extraction_errors_emits_learning_skipped_failed_and_completes`
//! ("turn completes despite extraction error (fail-soft)") — the last of which
//! also announced the skip.
//!
//! On the live path the pass is the plugin's `on_turn_finish`
//! (`plugins/database/mod.rs`) over `learn_after_turn` →
//! `run_post_turn_extraction`, which emits the extraction's usage only when the
//! provider reported one, announces `KnowledgeLearningSkipped` on a failed
//! call, and returns nothing that the turn could fail on. These are the same
//! three sentences, measured from the front's own stream on a composed chat
//! run whose extraction never answers.

use std::sync::Arc;
use std::time::Duration;

use saya_agent::ChatProvider;

use super::compose_fixture::Ceilings;
use super::learning_provider_fixture::{Scripted, composed};

/// Load ceiling, not a product bound: how long one scripted turn may take.
const RUN: Duration = Duration::from_secs(120);

/// The question: an assertion the learning gate treats as worth extracting.
const PROMPT: &str = "remember what the users table name column means";

/// One answered turn whose extraction call is made and never answered.
async fn turn_without_an_extraction_reply(
    case: &str,
) -> (
    crate::chat::ChatOutcome,
    Option<saya_agent::TokenUsage>,
    super::learning_provider_fixture::Recording,
    super::compose_fixture::RootGuard,
    crate::commands::coding::parent_turn_provider::Scripted,
) {
    let learning = Scripted::failing();
    let typed: Arc<dyn ChatProvider> = Arc::clone(&learning) as Arc<dyn ChatProvider>;
    let (composition, sink, root, provider) = composed(case, typed, Ceilings::default()).await;
    let output = tokio::time::timeout(RUN, composition.job().run_next_turn(&sink, PROMPT))
        .await
        .expect("the turn settles")
        .expect("an extraction failure never fails the turn: fail-soft");
    assert_eq!(
        learning.arrived(),
        1,
        "the extraction call was made, so its silence is what this test sees"
    );
    let outcome = crate::chat::ChatOutcome::from_stop(composition.job().turn_stop(), false);
    (outcome, output.learning_usage, sink, root, provider)
}

#[tokio::test]
async fn a_response_less_extraction_emits_no_usage_event() {
    let (outcome, learning_usage, sink, _root, _provider) =
        turn_without_an_extraction_reply("extraction-no-response").await;
    assert_eq!(
        outcome,
        crate::chat::ChatOutcome::Answered,
        "the turn answered, whatever the extraction did"
    );
    assert!(
        sink.learning_started(),
        "the extraction call was started, so there was really a call: {:?}",
        sink.events()
    );
    assert_eq!(
        sink.extraction_usage(),
        Vec::new(),
        "no response means no usage event: {:?}",
        sink.events()
    );
    assert_eq!(
        learning_usage, None,
        "the turn reports no learning usage for a call that never answered"
    );
}

/// G4 and G6 together: the extraction's error is announced, and the turn it
/// belonged to still ends with its answer.
#[tokio::test]
async fn an_extraction_error_never_fails_the_turn_and_announces_the_skip() {
    let (outcome, learning_usage, sink, _root, _provider) =
        turn_without_an_extraction_reply("extraction-error").await;
    assert_eq!(
        outcome,
        crate::chat::ChatOutcome::Answered,
        "turn completes despite extraction error (fail-soft)"
    );
    assert!(
        sink.learning_skipped(),
        "the skipped pass is announced rather than swallowed: {:?}",
        sink.events()
    );
    let skipped = sink
        .events()
        .into_iter()
        .filter(|event| {
            matches!(
                event,
                saya_agent::AgentEvent::KnowledgeLearningSkipped { .. }
            )
        })
        .count();
    assert_eq!(skipped, 1, "exactly one skip announcement");
    assert_eq!(
        learning_usage, None,
        "an error reports no usage: nothing was received to report"
    );
}
