//! The turn's output carries what the post-turn extraction spent.
//!
//! The old body reported the extraction call's own usage on the output it
//! returned (`AgentOutput::learning_usage`, "the extraction call's token
//! counts, reported separately from the answering total"), so `/usage` could
//! label the two calls apart.
//!
//! On the live path the pass is the database plugin's `on_turn_finish`, which
//! returns the usage it reported to the run. `parent_plugins::turn_finished`
//! discarded that return, so `AgentOutput::learning_usage` was `None` on every
//! live turn — a reported usage that no surface could read. This is the same
//! sentence on a composed chat run whose extraction answers and reports.

use std::sync::Arc;
use std::time::Duration;

use saya_agent::ChatProvider;

use super::compose_fixture::Ceilings;
use super::learning_provider_fixture::{Scripted, composed, usage};

/// Load ceiling, not a product bound: how long one scripted turn may take.
const RUN: Duration = Duration::from_secs(120);

/// The question: an assertion the learning gate treats as worth extracting.
const PROMPT: &str = "remember what the users table name column means";

#[tokio::test]
async fn an_answered_extraction_usage_rides_the_turns_output() {
    let learning = Scripted::answering();
    let typed: Arc<dyn ChatProvider> = Arc::clone(&learning) as Arc<dyn ChatProvider>;
    let (composition, sink, _root, _provider) =
        composed("extraction-usage", typed, Ceilings::default()).await;

    let output = tokio::time::timeout(RUN, composition.job().run_next_turn(&sink, PROMPT))
        .await
        .expect("the turn settles")
        .expect("an extraction failure never fails the turn: fail-soft");
    assert_eq!(
        learning.arrived(),
        1,
        "the extraction call was made, so its usage is what this test reads"
    );
    assert_eq!(
        sink.extraction_usage(),
        vec![usage()],
        "the stream reported the extraction's usage: {:?}",
        sink.events()
    );
    assert_eq!(
        output.learning_usage,
        Some(usage()),
        "the output carries what the extraction spent, apart from the answering total"
    );
    assert_eq!(
        crate::chat::ChatOutcome::from_stop(composition.job().turn_stop(), false),
        crate::chat::ChatOutcome::Answered,
        "the turn answered"
    );
}
