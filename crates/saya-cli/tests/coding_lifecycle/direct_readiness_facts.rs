//! The direct-mode advisory readiness carries the facts the readiness gives:
//! the changed paths with their kind, and the failed children with their class
//! bounded, redacted and carrying no authority. The `coding_status` tool does
//! not exist, so a test reads them through the `saya coding show` surface
//! (`job.status()`) once the run has settled. The readiness cache still holds
//! over them: a hit returns exactly what a fresh computation returns, and
//! a child that fails without an await — which moves no file and no receipt —
//! invalidates it through the children-state signature alone.

#[path = "direct_readiness_facts/support.rs"]
pub(crate) mod support;

use saya_agent::NoopEventSink;
use saya_types::JobState;
use serde_json::json;
use support::{
    Setup, completion_class, facts_job, latest_completion, readiness, run, wait_for_child_state,
};

#[tokio::test]
async fn readiness_lists_the_changed_paths_with_their_kind() {
    let (_fixture, job, _provider) = facts_job(
        Setup::new(
            "direct-facts-kinds",
            101,
            vec![
                "tracked.txt".into(),
                "added.txt".into(),
                "obsolete.txt".into(),
            ],
        )
        .baseline(vec![("obsolete.txt", "obsolete\n")]),
    )
    .await;

    run(&job).await;

    let readiness = readiness(&job).await;
    assert_eq!(
        readiness["changed_paths"],
        json!([
            {"path": "added.txt", "kind": "created"},
            {"path": "obsolete.txt", "kind": "deleted"},
            {"path": "tracked.txt", "kind": "modified"},
        ]),
        "every change W carries is listed once, sorted, with its kind: {readiness}"
    );
    assert_eq!(readiness["changed_paths_truncated"], false, "{readiness}");
    assert_eq!(readiness["w_changed"], true, "{readiness}");
}

#[tokio::test]
async fn changed_paths_are_bounded_and_flag_truncation() {
    // 65 changed paths: one more than the view lists.
    let paths = (0..65).map(|index| format!("f{index:02}.txt")).collect();
    let (_fixture, job, _provider) =
        facts_job(Setup::new("direct-facts-bounded", 102, paths)).await;

    run(&job).await;

    let readiness = readiness(&job).await;
    let changed = readiness["changed_paths"]
        .as_array()
        .unwrap_or_else(|| panic!("the readiness lists the changed paths: {readiness}"));
    assert_eq!(changed.len(), 64, "{readiness}");
    assert_eq!(changed[0]["path"], "f00.txt", "{readiness}");
    assert_eq!(changed[63]["path"], "f63.txt", "{readiness}");
    assert_eq!(changed[63]["kind"], "created", "{readiness}");
    assert_eq!(
        readiness["changed_paths_truncated"], true,
        "the view says it dropped paths: {readiness}"
    );
}

#[tokio::test]
async fn a_failed_child_is_listed_with_its_class() {
    let (fixture, job, _provider) = facts_job(
        Setup::new("direct-facts-child", 104, vec!["child-one.txt".into()]).retry(vec![5]),
    )
    .await;

    run(&job).await;

    let readiness = readiness(&job).await;
    assert_eq!(
        readiness["history_truncated"],
        json!(false),
        "a history the walk read to its end is not flagged: {readiness}"
    );
    let failures = readiness["child_failures"]
        .as_array()
        .unwrap_or_else(|| panic!("the readiness lists the child failures: {readiness}"));
    assert_eq!(failures.len(), 1, "{readiness}");
    assert_eq!(failures[0]["agent_id"], "child-one", "{readiness}");
    assert_eq!(failures[0]["outcome"], "failed", "{readiness}");
    let completion = latest_completion(&fixture, &job, "child-one").await;
    let recorded = completion_class(&completion);
    assert!(
        recorded.is_some(),
        "the failed child's own completion records a class"
    );
    assert_eq!(
        failures[0]["failure_class"].as_str(),
        recorded.map(|class| class.as_str()),
        "the readiness reports the class the child's own record carries: {readiness}"
    );
}

#[tokio::test]
async fn a_child_failing_without_an_await_invalidates_the_cached_facts() {
    let (fixture, job, provider) = facts_job(
        Setup::new("direct-facts-unawaited", 105, vec!["child-one.txt".into()]).retry(vec![5]),
    )
    .await;

    let sink = NoopEventSink;
    let running = job.run(&sink);
    tokio::pin!(running);
    let ((), output) = tokio::join!(
        async {
            // The script holds the second read until the child has actually
            // failed, so the turn after it sees a terminal child.
            wait_for_child_state(&fixture, &job, "child-one", JobState::Failed).await;
            provider.release();
        },
        tokio::time::timeout(support::RUN, &mut running),
    );
    output.expect("the scripted run terminates").unwrap();

    let after = readiness(&job).await;
    assert_eq!(
        after["child_failures"][0]["agent_id"], "child-one",
        "{after}"
    );
    assert_eq!(after["child_failures"][0]["outcome"], "failed", "{after}");
    // The failed child moved no file and no receipt, so the children-state
    // signature is the only term of the cache key that could have invalidated
    // the facts: had it not, the turn after the failure would have been a hit
    // and the count would be one lower.
    assert_eq!(
        job.readiness_computations(),
        2,
        "the child state alone invalidated the cached facts"
    );
}

#[tokio::test]
async fn a_cache_hit_still_equals_a_fresh_computation_with_the_new_facts() {
    let (_fixture, job, _provider) = facts_job(Setup::new(
        "direct-facts-hit",
        106,
        vec!["tracked.txt".into()],
    ))
    .await;

    run(&job).await;

    let fresh = readiness(&job).await;
    assert_eq!(
        fresh["changed_paths"],
        json!([{"path": "tracked.txt", "kind": "modified"}]),
        "the fresh computation carries the new facts: {fresh}"
    );
    assert_eq!(
        readiness(&job).await,
        fresh,
        "a cache hit returns exactly the fresh computation, new facts included"
    );
    // Three computations for a run that reads the checkout on every turn: the
    // first turn's, the advisory precompute started on the final turn's text,
    // and the outcome's after the owner wrote the prepared receipt. Every
    // later read is a hit.
    assert_eq!(
        job.readiness_computations(),
        3,
        "repeated reads of unchanged facts never compute again"
    );
}
