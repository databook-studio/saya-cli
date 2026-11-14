//! A repeated delegation is not refused by the loop: the loop keeps no repeat
//! refusal, so an identical call runs and reaches the delegation, which
//! refuses it for its own reason. The first repeat names a helper that has
//! never run, so it is admitted; the identical repeats name an agent already in
//! the task history, so the delegation refuses them — and the refusal is
//! proven before any effect, so nothing is left uncertain.
use super::{direct_fixture, tool_results, write_job};
use crate::provider::Provider;
use saya_agent::NoopEventSink;
use std::{sync::Arc, time::Duration};

/// Waits until the run has a live child, so the held child's request is what
/// the root is parked on.
async fn wait_for_live_child(job: &Arc<saya_cli::NativeCodingJob>) {
    tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            if !job.status().await.unwrap()["readiness"]["live_children"]
                .as_array()
                .unwrap()
                .is_empty()
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the root's first helper is live while its request is held");
}

#[tokio::test]
async fn an_identical_repeated_delegation_reaches_the_delegation_and_is_refused_by_it() {
    let provider = Provider::script(false, 94);
    let fixture = direct_fixture("converge-retry", &provider).await;
    let job = write_job(&fixture, "converge-retry", &["file-a.txt"]).await;
    let running = Arc::clone(&job);
    let run = tokio::spawn(async move { running.run(&NoopEventSink).await });

    wait_for_live_child(&job).await;
    provider.release();
    tokio::time::timeout(Duration::from_secs(60), run)
        .await
        .expect("the scripted run terminates")
        .unwrap()
        .expect("a refused delegation does not fail the job");

    let results = tool_results(&provider);
    // The first repeat names a helper that has never run, so it is admitted.
    assert_eq!(
        results["task-repeat"][0]["agent"], "child-b",
        "{}",
        results["task-repeat"]
    );
    assert_eq!(
        results["task-repeat"][0]["outcome"], "completed",
        "{}",
        results["task-repeat"]
    );
    // The identical repeats reach the delegation, which refuses them because
    // the agent is already recorded — never the loop's own repeat refusal.
    for tag in ["task-no-progress", "task-after-progress"] {
        let error = results[tag]["error"].as_str().unwrap_or_default();
        assert!(
            error.contains("job request is invalid"),
            "the identical repeat runs and the delegation refuses it: {tag}: {}",
            results[tag]
        );
        assert!(
            !error.contains("already failed") && !error.contains("identical"),
            "the loop never refuses a repeat: {tag}: {}",
            results[tag]
        );
    }
    assert!(job.failure().is_none(), "{:?}", job.failure());
    // Each repeat is refused by `validate_delegate_tasks` before any child is
    // registered, any checkout taken or any history line appended, so each
    // refusal proves no effect and leaves nothing to reconcile.
    assert_eq!(
        fixture
            .store
            .coding_recovery_status(job.job_id())
            .await
            .unwrap()
            .unresolved_reservations,
        0,
        "a refusal before any effect leaves nothing uncertain on record"
    );
}
