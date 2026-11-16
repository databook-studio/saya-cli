//! The job's one gate: concurrent helper asks queue, and a waiting ask still
//! honours cancellation.

use std::sync::Arc;
use std::time::Duration;

use saya_agent::CancellationToken;
use serde_json::json;

use super::support::{HELPER_LINE, HoldingOperator, RecordingOperator};
use crate::agent::tools::consent::{Operator, SerialisedOperator};

/// Two helpers asking at once are never pending at once: the job's one gate
/// serialises them, so the operator holds one ask at a time and both are
/// answered.
#[tokio::test]
async fn concurrent_helper_asks_are_never_pending_at_once() {
    let inner = RecordingOperator::new(true);
    let serialised: Arc<dyn Operator> = Arc::new(SerialisedOperator::new(
        inner.clone(),
        CancellationToken::new(),
    ));

    let first = {
        let serialised = Arc::clone(&serialised);
        tokio::spawn(async move { serialised.ask(HELPER_LINE, &json!({})).await })
    };
    let second = {
        let serialised = Arc::clone(&serialised);
        tokio::spawn(async move { serialised.ask(HELPER_LINE, &json!({})).await })
    };

    assert!(first.await.unwrap(), "the first ask is answered");
    assert!(second.await.unwrap(), "the second ask is answered");
    assert_eq!(inner.calls(), 2, "both asks reached the operator");
    assert_eq!(
        inner.peak(),
        1,
        "one prompt is pending at a time, whatever the helper"
    );
}

/// A waiting ask still honours cancellation: a cancelled job never holds the
/// gate and never reaches the operator.
#[tokio::test]
async fn a_waiting_ask_honours_cancellation() {
    let holding = HoldingOperator::new();
    let token = CancellationToken::new();
    let serialised: Arc<dyn Operator> =
        Arc::new(SerialisedOperator::new(holding.clone(), token.clone()));

    let first = {
        let serialised = Arc::clone(&serialised);
        tokio::spawn(async move { serialised.ask(HELPER_LINE, &json!({})).await })
    };
    // Let the first ask take the gate.
    for _ in 0..1_000 {
        if holding.entered() {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(holding.calls(), 1, "the first ask reached the operator");

    let waiting = {
        let serialised = Arc::clone(&serialised);
        tokio::spawn(async move { serialised.ask(HELPER_LINE, &json!({})).await })
    };
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(
        holding.calls(),
        1,
        "the second ask waits for the gate instead of asking at once"
    );

    token.cancel();
    assert!(
        !waiting.await.unwrap(),
        "a waiting ask is refused when the job is cancelled"
    );
    assert_eq!(
        holding.calls(),
        1,
        "the cancelled ask never reached the operator"
    );

    holding.release();
    assert!(first.await.unwrap(), "the ask that held the gate completes");
}
