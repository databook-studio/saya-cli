//! Direct-mode candidate preparation. There is no model-facing prepare tool:
//! the OWNER prepares the candidate at the end of the run from
//! the root working checkout W alone, with no children required. The prepared
//! receipt is bound to the current source generation, is immutable per
//! generation, and `candidate_ready` comes only from that receipt; status is
//! advisory.

#[path = "direct_prepare/fixture.rs"]
pub(crate) mod fixture;

use fixture::{
    checkout_path, checkouts_named, prepared_receipts, records_dir, resume, review_lines,
    run_direct_prepare_output,
};
use serde_json::Value;

/// The final result a write-path run ends with: the owner's readiness envelope.
fn final_envelope(output: &saya_agent::AgentOutput) -> Value {
    serde_json::from_str(&output.answer).expect("the run's answer is the readiness envelope")
}

#[tokio::test]
async fn a_root_only_edit_prepares_a_candidate_bound_to_the_working_source() {
    let (fixture, job, _provider, output) = run_direct_prepare_output(
        "direct-prepare-root",
        70,
        vec!["added.txt".into()],
        vec!["grep".into()],
    )
    .await;
    let receipt = final_envelope(&output);
    let digest = receipt["source_generation_sha256"]
        .as_str()
        .unwrap_or_else(|| panic!("the owner prepared a candidate: {receipt}"));
    assert_eq!(receipt["candidate_ready"], true, "{receipt}");
    assert_eq!(
        receipt["candidate"].as_str().unwrap(),
        format!("candidate-{digest}")
    );
    let candidate = checkout_path(&fixture, &job, receipt["candidate"].as_str().unwrap()).await;
    assert_eq!(
        std::fs::read_to_string(candidate.join("added.txt")).unwrap(),
        "FIXED\n",
        "the candidate carries W's delta"
    );
    assert!(
        records_dir(&fixture, &job)
            .join(format!("prepared-candidate-{digest}.json"))
            .exists(),
        "the receipt is durable for this generation"
    );
    assert_eq!(
        checkouts_named(&fixture, &job, "candidate-").await,
        1,
        "exactly the prepared candidate was created"
    );
    // An unchanged W replays the same receipt: a receipt is immutable per
    // generation, so a resumed run neither re-prepares nor reviews again.
    let resumed = resume(&fixture, &job).await;
    let replay = final_envelope(
        &tokio::time::timeout(
            std::time::Duration::from_secs(60),
            resumed.run(&saya_agent::NoopEventSink),
        )
        .await
        .expect("the resumed run terminates")
        .unwrap(),
    );
    assert_eq!(replay["candidate"], receipt["candidate"], "{replay}");
    assert_eq!(checkouts_named(&fixture, &job, "candidate-").await, 1);
    assert_eq!(prepared_receipts(&fixture, &job), 1);
    assert_eq!(review_lines(&fixture, &job), 1, "replay reruns no review");
}

#[tokio::test]
async fn an_edit_after_preparation_makes_the_receipt_stale() {
    // Mode 64: the first run's root writes W and answers, so the owner
    // prepares a candidate; the resumed run's root edits W again, so the
    // generation moves and the first receipt does not describe the checkout
    // the candidate is built from.
    let (fixture, job, _provider, output) = run_direct_prepare_output(
        "direct-prepare-stale",
        64,
        vec!["added.txt".into()],
        vec!["grep".into()],
    )
    .await;
    let first = final_envelope(&output);
    assert_eq!(first["candidate_ready"], true, "{first}");
    let first_digest = first["source_generation_sha256"]
        .as_str()
        .unwrap()
        .to_owned();
    let first_candidate = first["candidate"].as_str().unwrap().to_owned();

    let resumed = resume(&fixture, &job).await;
    let second = final_envelope(
        &tokio::time::timeout(
            std::time::Duration::from_secs(60),
            resumed.run(&saya_agent::NoopEventSink),
        )
        .await
        .expect("the resumed run terminates")
        .unwrap(),
    );
    assert_eq!(second["candidate_ready"], true, "{second}");
    assert_ne!(
        second["source_generation_sha256"].as_str().unwrap(),
        first_digest,
        "the further root edit moved the generation"
    );
    assert_ne!(second["candidate"].as_str().unwrap(), first_candidate);
    let second_digest = second["source_generation_sha256"].as_str().unwrap();
    assert!(
        records_dir(&fixture, &job)
            .join(format!("prepared-candidate-{second_digest}.json"))
            .exists(),
        "the fresh receipt is durable for the new generation"
    );
    let candidate = checkout_path(&fixture, &job, second["candidate"].as_str().unwrap()).await;
    assert_eq!(
        std::fs::read_to_string(candidate.join("added.txt")).unwrap(),
        "FIXED after the edit\n",
        "the new candidate carries the edit the first receipt never saw"
    );
    // The first receipt is immutable and stays on disk, but the readiness never
    // reports it: a receipt describes exactly one generation.
    assert!(
        records_dir(&fixture, &job)
            .join(format!("prepared-candidate-{first_digest}.json"))
            .exists()
    );
    let ready = crate::direct_task::readiness_now(&resumed).await;
    assert_eq!(ready["candidate"], second["candidate"], "{ready}");
    assert_eq!(ready["candidate_ready"], true, "{ready}");
}

#[tokio::test]
async fn direct_status_is_advisory_and_never_says_finish() {
    // A working checkout whose receipt the operator removed: the advisory view
    // reports the change and the next step, and forces no tool call.
    let (fixture, job, _provider, output) = run_direct_prepare_output(
        "direct-prepare-status",
        74,
        vec!["added.txt".into()],
        vec!["grep".into()],
    )
    .await;
    let prepared = final_envelope(&output);
    assert_eq!(prepared["candidate_ready"], true, "{prepared}");
    assert_eq!(prepared["suggestion"], "none", "{prepared}");
    assert!(prepared["next_action"].is_null(), "{prepared}");
    let digest = prepared["source_generation_sha256"].as_str().unwrap();
    std::fs::remove_file(
        records_dir(&fixture, &job).join(format!("prepared-candidate-{digest}.json")),
    )
    .unwrap();

    let working = crate::direct_task::readiness_now(&job).await;
    assert_eq!(working["candidate_ready"], false, "{working}");
    assert_eq!(working["w_changed"], true, "{working}");
    assert_eq!(
        working["suggestion"],
        "changes are in the workspace; run `saya job review direct-prepare-status` to prepare a candidate",
        "{working}"
    );
    assert!(job.status().await.unwrap()["next_action"].is_null());
}

#[tokio::test]
async fn a_headless_job_points_at_review_instead_of_a_preparation_that_never_runs() {
    let provider = crate::provider::Provider::before_images();
    let fixture = crate::support::Fixture::in_place_git_with_endpoint(
        "direct-prepare-deferred",
        &provider.endpoint,
    )
    .await;
    std::fs::write(fixture.repository.join("a.txt"), b"alpha\n").unwrap();
    std::fs::write(fixture.repository.join("old.txt"), b"old bytes\n").unwrap();
    let plan = saya_cli::launch_plan(saya_cli::OneShotMode::AcceptEdits, &[], false)
        .expect("accept-edits plans the whole folder");
    let job = saya_cli::oneshot_start(
        &fixture.application,
        saya_cli::OneShotRequest {
            task: "edit the plain folder".into(),
            job_id: Some("direct-prepare-deferred".into()),
            write_paths: plan.write_paths.clone(),
            mode: saya_cli::OneShotMode::AcceptEdits,
        },
        &plan,
        saya_agent::CancellationToken::new(),
    )
    .await
    .expect("the job starts");
    tokio::time::timeout(
        std::time::Duration::from_secs(60),
        job.run(&saya_agent::NoopEventSink),
    )
    .await
    .expect("the scripted run terminates")
    .unwrap();

    let ready = crate::direct_task::readiness_now(&job).await;
    assert_eq!(ready["w_changed"], true, "{ready}");
    assert_eq!(ready["candidate_ready"], false, "{ready}");
    assert_eq!(
        ready["suggestion"],
        "changes are in the workspace; run `saya job review direct-prepare-deferred` to prepare a candidate",
        "{ready}"
    );
    assert_eq!(
        prepared_receipts(&fixture, &job),
        0,
        "a headless job prepares nothing at the end of the run"
    );
}

#[tokio::test]
async fn a_direct_job_with_no_changes_prepares_nothing_and_says_so() {
    let (fixture, job, _provider, output) = run_direct_prepare_output(
        "direct-prepare-empty",
        73,
        vec!["added.txt".into()],
        vec!["grep".into()],
    )
    .await;
    let envelope = final_envelope(&output);
    assert_eq!(envelope["candidate_ready"], false, "{envelope}");
    assert_eq!(envelope["w_changed"], false, "{envelope}");
    assert_eq!(envelope["suggestion"], "none", "{envelope}");
    assert_eq!(
        prepared_receipts(&fixture, &job),
        0,
        "an unchanged working checkout prepares no candidate"
    );
    assert_eq!(
        checkouts_named(&fixture, &job, "candidate-").await,
        0,
        "an unchanged working checkout creates no candidate checkout"
    );
    assert!(
        crate::direct_task::root_checkout(&fixture, &job)
            .await
            .exists(),
        "the working checkout W itself still exists"
    );
}

#[tokio::test]
async fn a_second_prepare_without_a_delta_replays_the_receipt() {
    // Mode 110: the root's write is the generation the owner prepared; nothing
    // moves it afterwards, so a resumed run replays the receipt instead of
    // preparing again.
    let (fixture, job, _provider, output) = run_direct_prepare_output(
        "direct-prepare-delta",
        110,
        vec!["added.txt".into()],
        vec!["grep".into()],
    )
    .await;
    let first = final_envelope(&output);
    assert_eq!(first["candidate_ready"], true, "{first}");
    let resumed = resume(&fixture, &job).await;
    let second = final_envelope(
        &tokio::time::timeout(
            std::time::Duration::from_secs(60),
            resumed.run(&saya_agent::NoopEventSink),
        )
        .await
        .expect("the resumed run terminates")
        .unwrap(),
    );
    assert_eq!(
        second["candidate"], first["candidate"],
        "a generation with a receipt replays it: {second}"
    );
    assert_eq!(
        checkouts_named(&fixture, &job, "candidate-").await,
        1,
        "the replay creates no second candidate"
    );
    assert_eq!(prepared_receipts(&fixture, &job), 1);
    assert_eq!(
        review_lines(&fixture, &job),
        1,
        "the replay reruns no review"
    );
}
