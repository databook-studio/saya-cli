//! The advisory direct-mode readiness cache. The advisory facts reach the
//! coordinator as a continuation context block on every root turn, so repeated
//! turns with nothing changing compute W's delta and evidence binding ONCE;
//! every attempt that could have changed W — a successful write, a command
//! attempt, a helper's state moving — invalidates the cached facts; and a hit
//! returns exactly what a fresh computation returns. The retired
//! `coding_status` tool has no replacement, so a test reads the facts through
//! the `saya coding show` surface (`job.status()`) and counts computations.

#[path = "direct_status_cache/preparation.rs"]
mod preparation;
#[path = "direct_status_cache/support.rs"]
pub(crate) mod support;

use support::{READS, run, status_job, status_reads};

#[tokio::test]
async fn repeated_status_reads_with_no_change_compute_the_delta_once() {
    let (_fixture, job, provider) =
        status_job("direct-status-cache-repeats", 95, vec!["added.txt".into()]).await;

    run(&job).await;

    let reads = status_reads(&provider);
    assert_eq!(
        reads.len(),
        READS,
        "the scripted root read the checkout {READS} times"
    );
    let first = crate::direct_task::readiness_now(&job).await;
    assert_eq!(first["w_changed"], false, "{first}");
    assert_eq!(first["candidate_ready"], false, "{first}");
    assert_eq!(
        crate::direct_task::readiness_now(&job).await,
        first,
        "a cache hit returns exactly the facts a fresh computation returned"
    );
    assert_eq!(
        job.readiness_computations(),
        1,
        "a session of turns with nothing changing computes W's delta and evidence binding once"
    );
}

#[tokio::test]
async fn a_workspace_write_invalidates_the_cached_facts() {
    let (fixture, job, _provider) =
        status_job("direct-status-cache-write", 96, vec!["added.txt".into()]).await;

    run(&job).await;

    let after = crate::direct_task::readiness_now(&job).await;
    assert_eq!(after["w_changed"], true, "{after}");
    assert_eq!(after["candidate_ready"], true, "{after}");
    assert_eq!(
        std::fs::read_to_string(
            crate::direct_task::root_checkout(&fixture, &job)
                .await
                .join("added.txt")
        )
        .unwrap(),
        "FIXED\n",
        "the root's write landed in W"
    );
    // One computation for the turns before the write, one for the advisory
    // precompute started on the final turn's text, and one for the outcome
    // after the owner wrote the prepared receipt: the write moved W's
    // mutating-attempt epoch, which is the cache key, and the outcome's facts
    // are recomputed from the key the receipt write leaves behind.
    assert_eq!(
        job.readiness_computations(),
        3,
        "the write invalidated the cached facts"
    );
}

// The scripted attempt is a sandboxed `shell` call; only macOS proves that
// lane. On Linux the call is refused on the host lane, which the counterpart
// below pins.
#[cfg(target_os = "macos")]
#[tokio::test]
async fn a_command_attempt_that_changes_nothing_still_invalidates_the_cached_facts() {
    let (_fixture, job, provider) =
        status_job("direct-status-cache-command", 97, vec!["added.txt".into()]).await;

    run(&job).await;

    let attempt = crate::direct_task::tool_result(&provider, "call-refused-command");
    assert!(
        attempt.get("error").is_none(),
        "the scripted command attempt ran: {attempt}"
    );
    let after = crate::direct_task::readiness_now(&job).await;
    assert_eq!(
        after["w_changed"], false,
        "the command attempt changed nothing on disk: {after}"
    );
    // Two computations although W never moved: a `shell` call may have written
    // W whatever its result says, so the ATTEMPT alone invalidates the facts.
    assert_eq!(
        job.readiness_computations(),
        2,
        "the command attempt invalidated the cached facts"
    );
}

#[tokio::test]
async fn a_helper_that_changes_nothing_still_invalidates_the_cached_facts() {
    let (fixture, job, _provider) = status_job(
        "direct-status-cache-merge",
        98,
        vec!["added.txt".into(), "child-b.txt".into()],
    )
    .await;

    run(&job).await;

    let completions = fixture
        .store
        .list_coding_completion_history(job.job_id(), None, 32)
        .await
        .unwrap()
        .items;
    assert_eq!(
        completions.len(),
        1,
        "the child settled exactly one completion"
    );
    let after = crate::direct_task::readiness_now(&job).await;
    assert_eq!(
        after["w_changed"], false,
        "the child changed no file, so W did not move: {after}"
    );
    // Two computations although W never moved: the child's state moving is the
    // only term of the cache key that can carry a helper which wrote nothing.
    assert_eq!(
        job.readiness_computations(),
        2,
        "the child's state moving invalidated the cached facts"
    );
}

// Same sandboxed `shell` script as the test above, so the same gate applies.
#[cfg(target_os = "macos")]
#[tokio::test]
async fn a_cache_hit_equals_a_fresh_computation() {
    let (_fixture, job, _provider) =
        status_job("direct-status-cache-hit", 97, vec!["added.txt".into()]).await;

    run(&job).await;

    let fresh = crate::direct_task::readiness_now(&job).await;
    assert_eq!(
        crate::direct_task::readiness_now(&job).await,
        fresh,
        "a cache hit returns the facts a fresh computation returned"
    );
    assert_eq!(
        job.readiness_computations(),
        2,
        "two computations: the turn before the invalidating attempt, and the one after"
    );
}

// The Linux counterpart: no sandboxed command lane is offered, so the scripted
// `shell` attempt is refused with the documented message before it reaches the
// runner. Nothing runs, so nothing invalidates the cached facts.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn a_refused_command_attempt_leaves_the_cached_facts_intact() {
    let (_fixture, job, provider) = status_job(
        "direct-status-cache-command-host",
        97,
        vec!["added.txt".into()],
    )
    .await;

    run(&job).await;

    let attempt = crate::direct_task::tool_result(&provider, "call-refused-command");
    assert!(
        attempt["error"]
            .as_str()
            .unwrap_or_default()
            .contains("no operator is available"),
        "the command attempt is refused with the documented message: {attempt}"
    );
    let after = crate::direct_task::readiness_now(&job).await;
    assert_eq!(
        after["w_changed"], false,
        "a refused attempt changed nothing: {after}"
    );
    assert_eq!(
        job.readiness_computations(),
        1,
        "a refused attempt never reached the runner, so the cached facts stand"
    );
}
