//! The channel approval's ApprovalWait samples: one sample per entered wait
//! — a reply (allow or deny) is Succeeded, the cancellation is Cancelled, a
//! closed channel is Failed — and an automatic decision records nothing. The
//! samples ride a real [`ChannelApproval`] over a real channel, read back
//! through `LocalMetrics::snapshot()`.

use super::super::{ChannelApproval, StreamMsg};
use super::support::{read_shaped_tool, side_effecting_tool};
use crate::approval_facts::ApprovalFacts;
use crate::grant_token::TurnPrimary;
use crate::local_metrics::{LocalMetrics, outcome_index, phase_index};
use saya_agent::{
    AgentMetricOutcome, AgentMetricPhase, ApprovalChoice, ApprovalDecider, ApprovalPolicy,
    CancellationToken, SessionPolicy,
};
use std::sync::Arc;
use tokio::sync::mpsc::unbounded_channel;

/// The ApprovalWait cell of the collector's snapshot, by phase index.
fn wait_cell(metrics: &LocalMetrics) -> crate::local_metrics::PhaseCell {
    let snapshot = metrics.snapshot();
    snapshot.phases[phase_index(AgentMetricPhase::ApprovalWait).expect("known phase indexed")]
}

/// Pins exactly one ApprovalWait sample carrying the given outcome.
fn assert_one_sample(metrics: &LocalMetrics, outcome: AgentMetricOutcome) {
    let cell = wait_cell(metrics);
    assert_eq!(cell.samples, 1, "exactly one ApprovalWait sample");
    let slot = outcome_index(outcome).expect("known outcome indexed");
    assert_eq!(cell.outcomes[slot], 1, "the sample is {outcome:?}");
}

/// A reply — allow — succeeds the wait: exactly one ApprovalWait sample
/// with the Succeeded outcome.
#[tokio::test]
async fn an_allow_reply_records_one_succeeded_approval_wait_sample() {
    let tool = read_shaped_tool();
    let (tx, mut rx) = unbounded_channel();
    let metrics = Arc::new(LocalMetrics::new());
    let decider = ChannelApproval::with_cancellation(
        tx,
        SessionPolicy::new(ApprovalPolicy::Ask),
        TurnPrimary::default(),
        ApprovalFacts::default(),
        None,
        CancellationToken::new(),
    )
    .with_metrics(Some(metrics.clone()));
    let answerer = tokio::spawn(async move {
        if let Some(StreamMsg::ApprovalRequest { respond, .. }) = rx.recv().await {
            let _ = respond.send(ApprovalChoice::AllowOnce);
        }
    });
    assert!(
        decider
            .approve(&tool, &serde_json::json!({"sql": "SELECT 1"}))
            .await
    );
    answerer.await.expect("the answerer completes");
    assert_one_sample(&metrics, AgentMetricOutcome::Succeeded);
}

/// A deny also succeeds the wait — the wait succeeded; the user's answer
/// only decides the call. Still exactly one Succeeded sample.
#[tokio::test]
async fn a_deny_reply_still_records_a_succeeded_approval_wait_sample() {
    let tool = read_shaped_tool();
    let (tx, mut rx) = unbounded_channel();
    let metrics = Arc::new(LocalMetrics::new());
    let decider = ChannelApproval::with_cancellation(
        tx,
        SessionPolicy::new(ApprovalPolicy::Ask),
        TurnPrimary::default(),
        ApprovalFacts::default(),
        None,
        CancellationToken::new(),
    )
    .with_metrics(Some(metrics.clone()));
    let answerer = tokio::spawn(async move {
        if let Some(StreamMsg::ApprovalRequest { respond, .. }) = rx.recv().await {
            let _ = respond.send(ApprovalChoice::Deny);
        }
    });
    assert!(
        !decider
            .approve(&tool, &serde_json::json!({"sql": "SELECT 1"}))
            .await
    );
    answerer.await.expect("the answerer completes");
    assert_one_sample(&metrics, AgentMetricOutcome::Succeeded);
}

/// A live question cancellation wins a pending modal reply and ends the
/// wait in Cancelled: exactly one sample, recorded by the token's win —
/// mirroring `cancelled_channel_approval_denies_before_a_late_reply`.
#[tokio::test]
async fn a_cancelled_wait_records_one_cancelled_approval_wait_sample() {
    let tool = read_shaped_tool();
    let (tx, mut rx) = unbounded_channel();
    let metrics = Arc::new(LocalMetrics::new());
    let cancellation = CancellationToken::new();
    let decider = ChannelApproval::with_cancellation(
        tx,
        SessionPolicy::new(ApprovalPolicy::Ask),
        TurnPrimary::default(),
        ApprovalFacts::default(),
        None,
        cancellation.clone(),
    )
    .with_metrics(Some(metrics.clone()));
    let asking = tokio::spawn(async move { decider.approve(&tool, &serde_json::json!({})).await });
    let Some(StreamMsg::ApprovalRequest { respond, .. }) = rx.recv().await else {
        panic!("the ask must reach the real channel before cancellation");
    };
    cancellation.cancel();
    assert!(!asking.await.expect("the ask task joins"));
    assert_one_sample(&metrics, AgentMetricOutcome::Cancelled);
    assert!(
        respond.send(ApprovalChoice::AllowOnce).is_err(),
        "a late reply has no receiver after cancellation"
    );
}

/// An automatic decision — bypass allows the side-effecting call without
/// consulting the channel — never enters the wait: no request is sent and
/// no ApprovalWait sample exists.
#[tokio::test]
async fn an_automatic_decision_records_no_approval_wait_sample() {
    let tool = side_effecting_tool();
    let (tx, mut rx) = unbounded_channel();
    let metrics = Arc::new(LocalMetrics::new());
    let decider = ChannelApproval::with_cancellation(
        tx,
        SessionPolicy::new(ApprovalPolicy::Bypass),
        TurnPrimary::default(),
        ApprovalFacts::default(),
        None,
        CancellationToken::new(),
    )
    .with_metrics(Some(metrics.clone()));
    assert!(
        decider
            .approve(&tool, &serde_json::json!({"sql": "SELECT 1"}))
            .await,
        "bypass allows the side-effecting call without the channel"
    );
    assert!(
        rx.try_recv().is_err(),
        "an automatic decision never sends a request"
    );
    assert_eq!(
        wait_cell(&metrics).samples,
        0,
        "a wait never entered records no sample"
    );
}

/// A decider built with `with_metrics(None)` decides exactly as the
/// metric-less construction: the ask still rides the channel and the
/// reply still decides — the wait itself changes nothing.
#[tokio::test]
async fn metrics_none_decides_as_before_and_records_no_approval_wait() {
    let tool = read_shaped_tool();
    let (tx, mut rx) = unbounded_channel();
    let decider = ChannelApproval::with_cancellation(
        tx,
        SessionPolicy::new(ApprovalPolicy::Ask),
        TurnPrimary::default(),
        ApprovalFacts::default(),
        None,
        CancellationToken::new(),
    )
    .with_metrics(None);
    let answerer = tokio::spawn(async move {
        if let Some(StreamMsg::ApprovalRequest { respond, .. }) = rx.recv().await {
            let _ = respond.send(ApprovalChoice::AllowOnce);
        }
    });
    assert!(
        decider
            .approve(&tool, &serde_json::json!({"sql": "SELECT 1"}))
            .await
    );
    answerer.await.expect("the answerer completes");
}
