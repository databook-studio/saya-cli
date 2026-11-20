use super::*;
use crate::{BudgetDimension, CancellationReason};

/// A parent's stop reaches its child with the parent's own reason, and the
/// child's waiter wakes on it: a child waits exactly like a plain token.
#[tokio::test]
async fn a_childs_stop_is_its_parents_reason() {
    let parent = CancellationToken::new();
    let child = parent.child();
    assert!(
        !child.is_cancelled(),
        "a child runs while the token it was made from runs"
    );
    let waiter = child.clone();
    let waiting = tokio::spawn(async move { waiter.cancelled().await });
    tokio::task::yield_now().await;

    parent.cancel_with_reason(CancellationReason::Budget(BudgetDimension::ElapsedTime));

    tokio::time::timeout(Duration::from_millis(100), waiting)
        .await
        .expect("the child's waiter wakes when the parent stops")
        .expect("the waiter task joins");
    assert_eq!(
        child.reason(),
        Some(CancellationReason::Budget(BudgetDimension::ElapsedTime))
    );
}

/// Stopping a child is local: the token it was made from, and every sibling,
/// keep running.
#[test]
fn a_childs_stop_never_reaches_its_parent() {
    let parent = CancellationToken::new();
    let child = parent.child();
    let sibling = parent.child();

    assert!(child.cancel_with_reason(CancellationReason::Budget(BudgetDimension::LogicalRequests)));

    assert!(child.is_cancelled());
    assert_eq!(
        parent.reason(),
        None,
        "stopping a child leaves the token it came from running"
    );
    assert!(!parent.is_cancelled());
    assert!(!sibling.is_cancelled(), "one child's stop is not another's");
}

/// A child made from an already stopped token is stopped at creation, with the
/// reason that stopped the parent.
#[test]
fn a_child_of_a_stopped_parent_starts_stopped() {
    let parent = CancellationToken::new();
    parent.cancel_with_reason(CancellationReason::OwnerFailure);

    let child = parent.child();

    assert!(
        child.is_cancelled(),
        "a child born into a stopped run is stopped"
    );
    assert_eq!(child.reason(), Some(CancellationReason::OwnerFailure));
}

/// A parent tracks only the children that are still alive: ten thousand
/// children created and dropped leave it with nothing, and it still stops the
/// child made after them.
#[test]
fn dropped_children_do_not_accumulate_on_the_parent() {
    let parent = CancellationToken::new();
    let kept = parent.child();
    assert_eq!(
        parent.live_children(),
        1,
        "a live child is the one thing the parent tracks"
    );
    drop(kept);

    for _ in 0..10_000 {
        let child = parent.child();
        assert!(!child.is_cancelled(), "the parent has not stopped");
    }
    assert_eq!(
        parent.live_children(),
        0,
        "ten thousand dropped children leave no trace on the parent"
    );

    let last = parent.child();
    assert_eq!(parent.live_children(), 1);
    parent.cancel();
    assert!(
        last.is_cancelled(),
        "the pruning does not cost the parent its stop"
    );
}

/// A turn token armed on a run keeps a budget stop to itself — the run it was
/// armed for keeps running — while every other stop is the run's: a user stop
/// or an owner failure ends the run, and a stop of the run ends the turn.
#[test]
fn an_armed_turn_keeps_a_budget_stop_and_shares_the_runs_other_stops() {
    let run = CancellationToken::new();
    let turn = run.turn_of();
    assert!(
        turn.cancel_with_reason(CancellationReason::Budget(BudgetDimension::ReportedTokens)),
        "the turn takes the budget stop"
    );
    assert_eq!(
        run.reason(),
        None,
        "the budget stop stays in the turn it was armed for"
    );

    let failed_run = CancellationToken::new();
    let failed_turn = failed_run.turn_of();
    failed_turn.cancel_with_reason(CancellationReason::OwnerFailure);
    assert_eq!(
        failed_run.reason(),
        Some(CancellationReason::OwnerFailure),
        "an owner failure is the run's, exactly as it was on the run's own token"
    );

    let stopped_run = CancellationToken::new();
    let stopped_turn = stopped_run.turn_of();
    stopped_run.cancel();
    assert_eq!(stopped_turn.reason(), Some(CancellationReason::User));
}
