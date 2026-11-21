//! A deferred answering scope: it counts nothing until it is armed.

use saya_agent::{
    AgentError, BudgetDimension, CancellationReason, CancellationToken, DeferredAnsweringScope,
    DeferredCounts, NoopEventSink, TokenUsage, TurnPolicy,
};
use std::time::Duration;

fn requests_only(max_requests: u32) -> TurnPolicy {
    TurnPolicy {
        max_requests: Some(max_requests),
        ..TurnPolicy::default()
    }
}

fn usage(input: u64, output: u64) -> TokenUsage {
    TokenUsage::new(input, output)
}

/// Before arming, a turn's scope is a no-op: every admission succeeds and no
/// counter — requests, tool calls, known tokens, elapsed — moves.
#[tokio::test]
async fn an_unarmed_scope_admits_everything_and_counts_nothing() {
    let scope = DeferredAnsweringScope::new(CancellationToken::new());
    let sink = NoopEventSink;

    for _ in 0..100 {
        scope
            .admit_request(&sink)
            .await
            .expect("an unarmed scope refuses no request");
        scope
            .admit_tool_batch(2, &sink)
            .await
            .expect("an unarmed scope refuses no tool call");
        scope
            .settle_tokens(Some(usage(40, 60)))
            .await
            .expect("an unarmed scope refuses no known tokens");
    }

    assert_eq!(scope.counts().await, DeferredCounts::default());
    assert!(!scope.is_armed().await, "nothing armed this scope");
}

/// After `arm(requests: 1)` the second admission is refused with the
/// logical-requests dimension, and the stop lands on the turn token only: the
/// run's own token keeps running.
#[tokio::test]
async fn an_armed_scope_stops_its_turn_and_never_stops_the_run() {
    let run = CancellationToken::new();
    let turn = run.turn_of();
    let scope = DeferredAnsweringScope::new(turn.clone());
    let sink = NoopEventSink;

    scope.arm(requests_only(1)).await;
    scope
        .admit_request(&sink)
        .await
        .expect("the first request is inside the budget");
    let refused = scope
        .admit_request(&sink)
        .await
        .expect_err("the second request is outside it");

    assert!(
        matches!(refused, AgentError::Limit("answering budget stopped")),
        "the refusal is the answering budget's own error: {refused:?}"
    );
    assert_eq!(
        turn.reason(),
        Some(CancellationReason::Budget(BudgetDimension::LogicalRequests)),
        "the turn token carries the budget stop"
    );
    assert_eq!(
        run.reason(),
        None,
        "a budget stop is the turn's, never the run's"
    );
    assert_eq!(scope.counts().await.requests, 1, "one request was counted");
}

/// The elapsed dimension is measured from the arm instant, not from the turn's
/// start: five seconds of paused-clock time before arming are not charged.
#[tokio::test(start_paused = true)]
async fn elapsed_is_measured_from_the_arm_instant() {
    let scope = DeferredAnsweringScope::new(CancellationToken::new());

    tokio::time::advance(Duration::from_secs(5)).await;
    assert_eq!(
        scope.remaining_elapsed().await,
        None,
        "nothing is measured before the arm"
    );
    assert_eq!(scope.counts().await.elapsed, Duration::ZERO);

    scope
        .arm(TurnPolicy {
            max_elapsed: Some(Duration::from_secs(2)),
            ..TurnPolicy::default()
        })
        .await;
    assert_eq!(
        scope.remaining_elapsed().await,
        Some(Duration::from_secs(2)),
        "the clock starts at the arm, so the deadline is still whole"
    );

    tokio::time::advance(Duration::from_secs(2)).await;
    assert_eq!(
        scope.remaining_elapsed().await,
        Some(Duration::ZERO),
        "two seconds after arming the deadline has passed"
    );
    assert_eq!(
        scope.counts().await.elapsed,
        Duration::from_secs(2),
        "elapsed counts from the arm instant, never before it"
    );
}

/// Arming twice keeps the tightest ceiling per dimension, whichever order the
/// two arms arrive in.
#[tokio::test]
async fn arming_twice_keeps_the_tightest_policy() {
    let sink = NoopEventSink;
    let tight_first = DeferredAnsweringScope::new(CancellationToken::new());
    tight_first.arm(requests_only(5)).await;
    tight_first.arm(requests_only(2)).await;
    assert_eq!(admits_before_refusal(&tight_first, &sink).await, 2);

    let tight_second = DeferredAnsweringScope::new(CancellationToken::new());
    tight_second.arm(requests_only(2)).await;
    tight_second.arm(requests_only(5)).await;
    assert_eq!(admits_before_refusal(&tight_second, &sink).await, 2);
}

/// How many requests one scope admits before it refuses: the scope's effective
/// request ceiling.
async fn admits_before_refusal(scope: &DeferredAnsweringScope, sink: &NoopEventSink) -> u64 {
    let mut admitted = 0;
    while scope.admit_request(sink).await.is_ok() {
        admitted += 1;
        assert!(admitted < 100, "the ceiling never bound");
    }
    admitted
}
