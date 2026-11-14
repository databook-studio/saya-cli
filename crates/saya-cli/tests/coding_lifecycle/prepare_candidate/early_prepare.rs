use super::*;
use crate::direct_prepare::fixture::{checkouts_named, prepared_receipts, review_lines};

/// Waits until the store reports a live (non-terminal) child for `agent`.
async fn wait_for_live_child(fixture: &Fixture, job: &saya_cli::NativeCodingJob, agent: &str) {
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        let children = fixture
            .store
            .list_coding_children(job.job_id(), None, 100)
            .await
            .unwrap()
            .items;
        if children.iter().any(|child| {
            child.agent_id.as_str() == agent
                && !matches!(
                    child.state,
                    saya_types::JobState::Completed
                        | saya_types::JobState::Failed
                        | saya_types::JobState::Cancelled
                        | saya_types::JobState::Uncertain
                )
        }) {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the child {agent} never came live: {children:?}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn prepare_while_child_is_active_is_recoverable_and_replays_after_join() {
    let provider = Provider::direct_prepare_active_child();
    let fixture = direct_fixture("prepare-active-child", &provider).await;
    let job = job_with_paths(
        &fixture,
        "prepare-active-child",
        &["tracked.txt", "child-note.txt"],
    )
    .await;
    let run_job = Arc::clone(&job);
    let run = tokio::spawn(async move { run_job.run(&NoopEventSink).await });

    // The transport holds the child's first turn, so the root's synchronous
    // `helpers` call parks: the child is live and no candidate may exist yet.
    let live = tokio::time::timeout(Duration::from_secs(30), async {
        wait_for_live_child(&fixture, &job, "child-active").await;
        job.status().await.unwrap()
    })
    .await
    .expect("the child came live while the root waited on it");
    let readiness = &live["readiness"];
    assert_eq!(readiness["state"], "children_active", "{live}");
    assert_eq!(readiness["candidate_ready"], false, "{live}");
    assert_eq!(
        readiness["live_children"][0]["agent_id"], "child-active",
        "{live}"
    );
    assert_eq!(
        checkouts_named(&fixture, &job, "candidate-").await,
        0,
        "a live child admits no candidate"
    );
    assert_eq!(prepared_receipts(&fixture, &job), 0);

    provider.release();
    tokio::time::timeout(Duration::from_secs(60), run)
        .await
        .unwrap()
        .unwrap()
        .unwrap();

    let settled = job.status().await.unwrap();
    let readiness = &settled["readiness"];
    assert_eq!(readiness["state"], "candidate_ready", "{settled}");
    assert_eq!(readiness["candidate_ready"], true, "{settled}");
    assert_eq!(
        readiness["live_children"],
        serde_json::json!([]),
        "{settled}"
    );
    assert_eq!(readiness["suggestion"], "none", "{settled}");
    assert_eq!(
        checkouts_named(&fixture, &job, "candidate-").await,
        1,
        "exactly the prepared candidate was created"
    );
    assert_eq!(prepared_receipts(&fixture, &job), 1);
    assert_eq!(review_lines(&fixture, &job), 1);
    assert_eq!(
        fs::read(fixture.repository.join("tracked.txt")).unwrap(),
        b"original\n",
        "candidate preparation never changes the original tree"
    );

    // The join settled the child; a resumed run replays the receipt instead of
    // preparing again.
    let (resumed, output) = run_resumed(&fixture, &job).await;
    let replay = final_envelope(&output);
    assert_eq!(
        replay["candidate"], settled["readiness"]["candidate"],
        "{replay}"
    );
    assert_eq!(
        checkouts_named(&fixture, &job, "candidate-").await,
        1,
        "the replay creates no checkout"
    );
    assert_eq!(prepared_receipts(&fixture, &job), 1);
    assert_eq!(
        review_lines(&fixture, &job),
        1,
        "the replay reruns no review"
    );
    assert_eq!(
        resumed.status().await.unwrap()["readiness"]["candidate_ready"],
        true
    );
}
