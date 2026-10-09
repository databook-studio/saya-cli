use super::*;

/// The child forks a detached grandchild and stays alive; the timeout
/// kills the group and the grandchild goes with it. `process-fork` had
/// to be granted with a measured reason for this child to fork at all —
/// the battery provision carries it.
#[tokio::test]
async fn a_timeout_kills_a_daemonizing_grandchild() {
    let outcome = with_fork()
        .tool()
        .run(PROGRAM, &["fork-daemon".to_owned()], Some(2))
        .await
        .expect("the run itself must complete");
    assert!(
        outcome.killed_by_timeout,
        "the timeout must have fired: {outcome:?}"
    );
    assert_eq!(outcome.exit_code, None, "a killed child has no exit code");
    assert!(
        wait_group_gone(outcome.pid),
        "the daemonizing grandchild must be gone with the group"
    );
}

/// The sibling escape: the child exits 0 and leaves the detached
/// grandchild running. The timeout never fires — so the runner sweeps
/// the group after the child is reaped, and reports it.
#[tokio::test]
async fn a_child_that_exits_leaving_an_orphan_is_swept_and_reported() {
    let outcome = with_fork()
        .tool()
        .run(PROGRAM, &["fork-orphan".to_owned()], None)
        .await
        .expect("the run itself must complete");
    assert_eq!(outcome.exit_code, Some(0), "the parent exited normally");
    assert!(
        outcome.killed_orphans,
        "the orphaned grandchild must be reported as swept: {outcome:?}"
    );
    assert!(
        wait_group_gone(outcome.pid),
        "the detached grandchild must not survive the child"
    );
}

/// Cancellation kills the process group with the same killpg path the
/// timeout uses, and the outcome says it was cancelled.
#[tokio::test]
async fn cancellation_kills_the_process_group() {
    let token = saya_agent::CancellationToken::new();
    let tool = standard().tool().with_cancellation(token.clone());
    let run = tokio::spawn(async move {
        tool.run(PROGRAM, &["sleep".to_owned(), "30".to_owned()], None)
            .await
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    token.cancel();
    let outcome = run
        .await
        .expect("the run task must finish")
        .expect("cancel reports the child");
    assert!(
        outcome.cancelled,
        "the cancellation must be reported: {outcome:?}"
    );
    assert!(
        !outcome.killed_by_timeout,
        "a cancelled child is not a timed-out child: {outcome:?}"
    );
    assert!(wait_group_gone(outcome.pid), "the group must be gone");
}
