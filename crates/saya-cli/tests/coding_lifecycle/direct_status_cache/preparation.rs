//! Preparation never reads the advisory readiness cache: a change made
//! directly in the working checkout, behind every counter the key holds, must
//! still be prepared by the owner at the end of the run. The retired
//! `coding_prepare_candidate` tool has no replacement, so only the owner's
//! preparation is left to assert.

use super::support::{
    BEHIND_THE_CACHE, READS, prepared_generations, resume, run, run_behind_the_cache, status_job,
};
use crate::direct_prepare::fixture::checkout_path;

#[tokio::test]
async fn the_owner_preparation_never_uses_the_cache() {
    // The out-of-band write lands in W between two turns whose context blocks
    // are cache hits, so the advisory facts keep saying nothing changed; the
    // owner's preparation at the end of the run still sees the change.
    let (fixture, job, provider) =
        status_job("direct-status-cache-finish", 100, vec!["extra.txt".into()]).await;
    run_behind_the_cache(&fixture, &job, &provider).await;

    // The advisory facts every turn before the owner prepared were cache hits
    // that never saw the write: they still report an unchanged checkout.
    let cached = crate::direct_task::readiness_now(&job).await;
    assert_eq!(
        cached["w_changed"], false,
        "the cache is blind to the out-of-band write: {cached}"
    );
    let generations = prepared_generations(&fixture, &job);
    assert_eq!(
        generations.len(),
        1,
        "the owner prepared exactly one candidate at the end of the run"
    );
    assert_ne!(
        generations[0],
        cached["source_generation_sha256"].as_str().unwrap(),
        "the owner's preparation bound the change the cached facts never saw"
    );
    let candidate = checkout_path(&fixture, &job, &format!("candidate-{}", generations[0])).await;
    assert_eq!(
        std::fs::read_to_string(candidate.join("extra.txt")).unwrap(),
        BEHIND_THE_CACHE,
        "the owner's candidate carries the change only the checkout has"
    );
}

#[tokio::test]
async fn a_resumed_job_reuses_the_cached_facts_for_repeated_status_reads() {
    let (fixture, job, _provider) =
        status_job("direct-status-cache-timing", 95, vec!["added.txt".into()]).await;
    run(&job).await;
    let job_id = job.job_id().clone();

    // A resumed job starts with an empty cache over the same working checkout,
    // so its first status read costs exactly what every read cost before the
    // cache existed.
    let resumed = resume(&fixture, &job_id).await;
    let started = std::time::Instant::now();
    resumed.status().await.unwrap();
    let cold = started.elapsed();
    let started = std::time::Instant::now();
    for _ in 0..READS {
        resumed.status().await.unwrap();
    }
    let warm = started.elapsed() / READS as u32;
    assert_eq!(
        resumed.readiness_computations(),
        1,
        "{READS} warm reads over an unchanged checkout compute nothing"
    );
    eprintln!(
        "cold status read {cold:?}, warm {warm:?} ({}x)",
        cold.as_secs_f64() / warm.as_secs_f64().max(f64::EPSILON)
    );
}
