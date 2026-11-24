//! The turn's end is stated to the plugins after the answering clock is done:
//! a slow extraction is awaited, and the answering deadline never cuts it.
//!
//! The old body's two pins said this from the inside of one timer:
//! `answering_deadline_disarms_before_source_observed_extraction` held the
//! extraction source past the answering budget and read
//! `assert_eq!(cancellation.reason(), None, "the answering clock must be
//! disarmed while extraction is pending")`, and `a_slow_extraction_is_awaited_
//! not_cut_off` held a slow reply and read `"the call was genuinely awaited,
//! not raced against a timeout"`.
//!
//! On the live path the shape is the same and structural: the turn's loop is
//! raced against its armed budget (`commands/coding/parent_loop.rs`'s
//! `select!`), and only once that race has ended is the turn's end stated to
//! the plugins (`parent_plugins::turn_finished`), whose learning pass awaits
//! the extraction call. These two tests are that sentence measured from
//! outside: the turn is still running while the extraction is pending, and the
//! turn's own budget stopped nothing.

use std::sync::Arc;
use std::time::Duration;

use saya_agent::ChatProvider;

use super::compose_fixture::Ceilings;
use super::learning_provider_fixture::{Scripted, composed, usage};

/// Load ceiling, not a product bound: how long one scripted turn may take.
const RUN: Duration = Duration::from_secs(120);

/// The question: an assertion the learning gate treats as worth extracting.
const PROMPT: &str = "remember what the users table name column means";

/// The one database budget this file's disarm case states: one second, so an
/// answering clock still armed would cut an extraction held past it.
const ELAPSED_SECONDS: u64 = 1;

#[tokio::test]
async fn a_slow_extraction_is_awaited_not_raced() {
    let learning = Scripted::held();
    let typed: Arc<dyn ChatProvider> = Arc::clone(&learning) as Arc<dyn ChatProvider>;
    let (composition, sink, _root, _provider) =
        composed("extraction-awaited", typed, Ceilings::default()).await;

    let work = composition.job().run_next_turn(&sink, PROMPT);
    tokio::pin!(work);
    tokio::select! {
        () = learning.wait_arrived() => {}
        result = &mut work => panic!(
            "the turn settled before the post-turn extraction asked: {result:?}; events: {:?}",
            sink.events()
        ),
    }
    // The answer is on screen and the extraction call is in flight: a pass
    // raced against a timeout would settle the turn now.
    tokio::select! {
        () = tokio::time::sleep(Duration::from_millis(400)) => {}
        result = &mut work => panic!(
            "the turn settled while the extraction was still pending: {result:?}"
        ),
    }
    assert_eq!(learning.arrived(), 1, "one extraction call, still awaited");

    learning.release();
    let _output = tokio::time::timeout(RUN, work)
        .await
        .expect("the turn settles once the extraction answers")
        .expect("the turn must run");
    assert_eq!(
        sink.extraction_usage(),
        vec![usage()],
        "the extraction's own report reached the stream after it answered"
    );
}

#[tokio::test]
async fn the_answering_clock_is_disarmed_while_extraction_is_pending() {
    let learning = Scripted::held();
    let typed: Arc<dyn ChatProvider> = Arc::clone(&learning) as Arc<dyn ChatProvider>;
    let ceilings = Ceilings {
        elapsed_seconds: Some(ELAPSED_SECONDS),
        ..Ceilings::default()
    };
    let (composition, sink, _root, _provider) =
        composed("extraction-disarmed", typed, ceilings).await;

    let work = composition.job().run_next_turn(&sink, PROMPT);
    tokio::pin!(work);
    tokio::select! {
        () = learning.wait_arrived() => {}
        result = &mut work => panic!(
            "the turn settled before the post-turn extraction asked: {result:?}"
        ),
    }
    // Held past the one-second answering budget: an ancillary clock still armed
    // would stop the turn here, and the turn would settle.
    tokio::select! {
        () = tokio::time::sleep(Duration::from_millis(1_400)) => {}
        result = &mut work => panic!(
            "the answering clock cut a pending extraction: {result:?}"
        ),
    }
    assert_eq!(
        sink.budget_stops(),
        Vec::new(),
        "the answering clock reported no stop while extraction was pending: {:?}",
        sink.budget_stops()
    );

    learning.release();
    let _output = tokio::time::timeout(RUN, work)
        .await
        .expect("the turn settles once the extraction answers")
        .expect("the turn must run");
    assert_eq!(
        sink.extraction_usage(),
        vec![usage()],
        "the ancillary call neither consumed nor was cut by answering time"
    );
    assert_eq!(
        sink.budget_stops(),
        Vec::new(),
        "ancillary extraction does not add an answering budget stop: {:?}",
        sink.budget_stops()
    );
}
