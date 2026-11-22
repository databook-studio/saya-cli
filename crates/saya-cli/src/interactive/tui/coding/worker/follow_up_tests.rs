use super::{CodingMessage, PanelSenders, work};
use crate::CodingCommand;
use crate::coding_test_support::{follow_up_fixture, follow_up_provider};
use saya_agent::CancellationToken;
use std::{
    fs,
    sync::{atomic::Ordering, mpsc},
    time::{Duration, Instant},
};
const TEST: &str = "interactive::tui::coding::worker::follow_up_tests::inactive_follow_up_delivers_child_controls_and_progress";
#[test]
fn inactive_follow_up_delivers_child_controls_and_progress() {
    if crate::coding_test_support::isolation::run_parent("tui", TEST) {
        return;
    }
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(tokio::task::LocalSet::new().run_until(exercise()));
}

async fn exercise() {
    let root = crate::state_path::state_db_path()
        .parent()
        .unwrap()
        .to_path_buf();
    let provider = follow_up_provider::Provider::new();
    let fixture = follow_up_fixture::Fixture::new(&root, &provider).await;
    let completion = &fixture.completion;
    assert!(!fixture.output.truncated);
    assert_eq!(fixture.job.job_id(), completion.result.job_id());
    let command = CodingCommand::FollowUp {
        id: completion.result.job_id().as_str().into(),
        agent: completion.result.agent_id().as_str().into(),
        expected_completion_sequence: completion.sequence.get(),
        instruction: "repair only the original owned file".into(),
        approve: true,
        in_place: false,
        managed: false,
    };
    let token = CancellationToken::new();
    let (activity, activity_rx) = mpsc::sync_channel(128);
    let (receipts, receipt_rx) = mpsc::sync_channel(64);
    let (controls, control_rx) = tokio::sync::mpsc::channel(16);
    let work = work(
        command,
        fixture.runtime.clone(),
        fixture.store.clone(),
        fixture.repository.clone(),
        token.clone(),
        PanelSenders { activity, receipts },
        control_rx,
        saya_agent::ApprovalPolicy::Ask,
    );
    let task = tokio::task::spawn_local(work);
    let started = wait_for(
        || provider.started.load(Ordering::Acquire),
        Duration::from_secs(30),
    )
    .await;
    if started {
        controls
            .send(CodingCommand::CancelChild {
                id: completion.result.job_id().as_str().into(),
                agent: completion.result.agent_id().as_str().into(),
            })
            .await
            .unwrap();
    }
    // Receipts and progress travel on independent channels, so neither their
    // order nor their arrival before the next poll is guaranteed. Wait for
    // both within one bounded window instead of sampling once.
    let mut receipt = None;
    let mut progress = false;
    if started {
        // Below the fixture's 30 s provider timeout, so a slow delivery is
        // still observed as a delivery failure, not as a timed-out request.
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline && (receipt.is_none() || !progress) {
            while let Ok(message) = receipt_rx.try_recv() {
                if let CodingMessage::Activity(text) = message
                    && text.contains("cancel_requested")
                {
                    receipt = Some(text);
                }
            }
            while let Ok(message) = activity_rx.try_recv() {
                if let CodingMessage::Activity(text) = message
                    && text.contains(completion.result.agent_id().as_str())
                {
                    progress = true;
                }
            }
            if receipt.is_none() || !progress {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
    }
    token.cancel();
    provider.release.store(true, Ordering::Release);
    let settled = tokio::time::timeout(Duration::from_secs(15), task).await;
    assert!(started, "actual new child reached held provider request");
    assert!(
        settled.as_ref().is_ok_and(|result| result.is_ok()),
        "worker joined after fixture cleanup: {settled:?}"
    );
    assert!(
        receipt
            .as_deref()
            .is_some_and(|text| text.contains("cancel_requested")),
        "inactive follow-up must execute queued child control: {receipt:?}"
    );
    assert!(
        progress,
        "inactive follow-up must forward actual child progress"
    );
    assert_eq!(
        fs::read(fixture.repository.join("tracked.txt")).unwrap(),
        b"original\n"
    );
    let job = fixture
        .store
        .get_coding_job(completion.result.job_id())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(
            job.state,
            saya_types::JobState::Cancelled | saya_types::JobState::Failed
        ),
        "accepted cancellation run must persist its terminal outcome"
    );
}
async fn wait_for(mut ready: impl FnMut() -> bool, limit: Duration) -> bool {
    let started = Instant::now();
    while started.elapsed() < limit {
        if ready() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    false
}
