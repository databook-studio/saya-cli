//! The elapsed dimension of a plugin's policy, armed on first use (C43).
//!
//! The ceiling is a deadline on the tokio clock anchored at the arm, never at
//! the turn's start: a policy of 8 s armed 5 s into a turn stops it at 13 s. A
//! turn that never arms one waits on nothing, and the stop it latches is the
//! turn's own, so the run keeps taking turns.

#![cfg(unix)]

use super::parent_arming::{TurnArming, stop_the_turn};
use super::parent_arming_fixture::{TestPlugin, active, first_turn, inputs};
use super::parent_turn_fixture::in_place_writing;
use super::parent_turn_provider::Scripted;
use saya_agent::{BudgetDimension, CancellationToken, TurnPolicy};
use std::sync::Arc;
use std::time::Duration;

/// A policy with one dimension: the longest the turn may run once armed.
fn elapsed(seconds: u64) -> TurnPolicy {
    TurnPolicy {
        max_elapsed: Some(Duration::from_secs(seconds)),
        ..TurnPolicy::default()
    }
}

/// Test 4 — 5 s of the turn pass, the 8 s policy arms, and the deadline passes
/// at 13 s, not at the turn's own 8 s mark.
#[tokio::test(start_paused = true)]
async fn the_elapsed_deadline_starts_at_the_arm_not_at_the_turn_start() {
    let plugin = Arc::new(TestPlugin::new(elapsed(8), false));
    let turn = CancellationToken::new();
    let arming = TurnArming::new(Some(active(plugin)), turn.clone());
    let mut expired = std::pin::pin!(arming.expire());

    // Five seconds of the turn pass before anything uses the plugin: an unarmed
    // turn has no elapsed budget at all.
    tokio::select! {
        biased;
        () = &mut expired => panic!("a turn that has not used the plugin has no elapsed budget"),
        () = tokio::time::sleep(Duration::from_secs(5)) => {}
    }
    arming.arm_on_route().await;

    // At the turn's own 8 s mark — three seconds after the arm — nothing fires.
    tokio::select! {
        biased;
        () = &mut expired => panic!("the deadline is the arm's + 8 s, not the turn's first 8 s"),
        () = tokio::time::sleep(Duration::from_secs(3)) => {}
    }
    // Eight seconds after the arm it does, and the stop is the turn's own.
    tokio::select! {
        biased;
        () = &mut expired => {}
        () = tokio::time::sleep(Duration::from_secs(6)) => panic!("the armed deadline never fired"),
    }
    assert_eq!(
        arming.stop_dimension(),
        None,
        "no budget stop has latched yet"
    );
    stop_the_turn(&turn);
    assert_eq!(
        arming.stop_dimension(),
        Some(BudgetDimension::ElapsedTime),
        "the elapsed stop lands on the turn's own token"
    );
}

/// Test 4b — the same deadline in a live turn: the request the provider holds
/// open is stopped by the armed elapsed budget, and the run stays open.
#[tokio::test]
async fn an_armed_elapsed_budget_stops_a_held_request() {
    let provider = Scripted::start();
    provider.hold_first();
    let fixture = in_place_writing("arming-elapsed", &provider.endpoint).await;
    let plugin = Arc::new(TestPlugin::new(elapsed(1), true));

    let stopped = first_turn(&fixture.job, inputs(plugin)).await;
    provider.release();
    assert!(
        stopped.is_ok(),
        "a spent budget is a partial answer: {stopped:?}"
    );
    assert!(provider.arrived() >= 1, "the request reached the provider");
    let stop = fixture
        .job
        .turn_stop()
        .expect("the turn states what stopped it");
    assert_eq!(
        stop.message(),
        "the run stopped on its elapsed-time budget; any answer it produced is partial or absent"
    );
    assert_eq!(
        fixture.job.cancellation.reason(),
        None,
        "the deadline stopped the turn's token, never the run's"
    );
    assert!(fixture.job.holds_folder_lease(), "the run is still open");
}
