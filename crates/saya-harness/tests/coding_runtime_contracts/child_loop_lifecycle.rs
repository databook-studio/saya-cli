#[path = "child_loop_lifecycle_support.rs"]
mod support;

use std::sync::Arc;
use std::sync::atomic::Ordering;

use saya_agent::{CancellationReason, CancellationToken};
use saya_harness::coding::coordinator::{
    ChildEventKind, ChildProgress, ChildRunError, MAX_CHILD_EVENT_BYTES,
    MAX_CHILD_EVENT_QUEUE_CAPACITY, child_event_channel, run_child,
};

use support::{ScenarioFactory, request, spec};

async fn drain(
    mut receiver: saya_harness::coding::coordinator::ChildEventReceiver,
) -> Vec<saya_harness::coding::coordinator::ChildEvent> {
    let mut events = Vec::new();
    while let Some(event) = receiver.recv().await {
        events.push(event);
    }
    events
}

#[tokio::test]
async fn child_factory_receives_narrowed_spec_and_runs_existing_loop() {
    let factory = ScenarioFactory::new(1);
    let spec = spec("agent-a", "invocation-a", request(), 2);
    let authority = spec.authority();
    let expected_job = authority.job_id().clone();
    let expected_agent = authority.agent_id().clone();
    let expected_invocation = authority.invocation_id().clone();
    let expected_repository = authority.repository_id().clone();
    let expected_snapshot = authority.snapshot_id().clone();
    let (sender, receiver) = child_event_channel(&spec);
    let cancellation = CancellationToken::new();

    let (output, events) = tokio::join!(
        run_child(spec, &factory, sender, cancellation),
        drain(receiver)
    );
    let output = output.unwrap();

    assert_eq!(output.answer, "child answer");
    assert_eq!(factory.stats.factory_calls.load(Ordering::SeqCst), 1);
    assert_eq!(factory.stats.provider_calls.load(Ordering::SeqCst), 2);
    assert_eq!(factory.stats.executor_calls.load(Ordering::SeqCst), 1);
    let captured = factory.stats.captured.lock().unwrap();
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].job_id, expected_job.as_str());
    assert_eq!(captured[0].agent_id, expected_agent.as_str());
    assert_eq!(captured[0].invocation_id, expected_invocation.as_str());
    assert_eq!(captured[0].repository_id, expected_repository.as_str());
    assert_eq!(captured[0].snapshot_id, expected_snapshot.as_str());
    assert_eq!(captured[0].checkout_id.as_deref(), Some("checkout-a"));
    assert_eq!(captured[0].write_paths, vec!["src/owned.rs".to_owned()]);
    assert!(!captured[0].workspace_write);
    drop(captured);

    assert!(events.iter().any(|event| {
        matches!(
            event.kind(),
            ChildEventKind::Progress {
                progress: ChildProgress::ToolRequested { .. }
            }
        )
    }));
    assert!(events.iter().any(|event| {
        matches!(
            event.kind(),
            ChildEventKind::Progress {
                progress: ChildProgress::ToolCompleted
            }
        )
    }));
    assert!(
        events
            .iter()
            .any(|event| { matches!(event.kind(), ChildEventKind::Observation { .. }) })
    );
    let tool_requested = events
        .iter()
        .position(|event| {
            matches!(
                event.kind(),
                ChildEventKind::Progress {
                    progress: ChildProgress::ToolRequested { .. }
                }
            )
        })
        .expect("the tool request is projected");
    let tool_completed = events
        .iter()
        .position(|event| {
            matches!(
                event.kind(),
                ChildEventKind::Progress {
                    progress: ChildProgress::ToolCompleted
                }
            )
        })
        .expect("tool completion is projected");
    let complete = events
        .iter()
        .position(|event| {
            matches!(
                event.kind(),
                ChildEventKind::Progress {
                    progress: ChildProgress::Complete
                }
            )
        })
        .expect("completion is projected");
    assert!(tool_requested < tool_completed && tool_completed < complete);
    assert_eq!(complete, events.len() - 1);
    for event in events {
        assert_eq!(event.job_id(), &expected_job);
        assert_eq!(event.agent_id(), &expected_agent);
        assert_eq!(event.invocation_id(), &expected_invocation);
        let bytes = event.to_json_bytes().unwrap();
        assert!(bytes.len() <= MAX_CHILD_EVENT_BYTES);
        let projected = String::from_utf8(bytes).unwrap();
        assert!(!projected.contains("PRIVATE-TOOL-ARG-SENTINEL"));
        assert!(!projected.contains("PRIVATE-TOOL-RESULT-SENTINEL"));
        assert!(!projected.contains("child answer"));
    }
}

#[tokio::test]
async fn independent_children_get_fresh_answering_and_cancellation_scopes() {
    let factory = ScenarioFactory::new(1);
    let first_token = CancellationToken::new();
    let first = spec("agent-a", "invocation-a", request(), 2);
    let (first_sender, first_receiver) = child_event_channel(&first);
    let (first_result, _) = tokio::join!(
        run_child(first, &factory, first_sender, first_token.clone()),
        drain(first_receiver)
    );
    assert_eq!(first_result.unwrap().answer, "child answer");
    first_token.cancel_with_reason(CancellationReason::User);

    let second_token = CancellationToken::new();
    let second = spec("agent-b", "invocation-b", request(), 2);
    let (second_sender, second_receiver) = child_event_channel(&second);
    let (second_result, _) = tokio::join!(
        run_child(second, &factory, second_sender, second_token.clone()),
        drain(second_receiver)
    );
    assert_eq!(second_result.unwrap().answer, "child answer");

    assert_eq!(factory.stats.provider_calls.load(Ordering::SeqCst), 4);
    assert!(first_token.is_cancelled());
    assert!(!second_token.is_cancelled());
}

#[tokio::test]
async fn closed_receiver_becomes_owner_failure_instead_of_success() {
    let factory = ScenarioFactory::new(1);
    let spec = spec("agent-a", "invocation-a", request(), 2);
    let (sender, receiver) = child_event_channel(&spec);
    drop(receiver);
    let cancellation = CancellationToken::new();

    let result = run_child(spec, &factory, sender, cancellation.clone()).await;

    assert!(matches!(result, Err(ChildRunError::OwnerFailure)));
    assert_eq!(
        cancellation.reason(),
        Some(CancellationReason::OwnerFailure)
    );
}

#[tokio::test]
async fn event_channel_identity_must_match_child_spec() {
    let factory = ScenarioFactory::new(1);
    let child_spec = spec("agent-a", "invocation-a", request(), 2);
    let other_spec = spec("agent-b", "invocation-b", request(), 2);
    let (sender, _receiver) = child_event_channel(&other_spec);

    let result = run_child(child_spec, &factory, sender, CancellationToken::new()).await;

    assert!(matches!(
        result,
        Err(ChildRunError::ChannelIdentityMismatch)
    ));
    assert_eq!(factory.stats.factory_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn full_progress_channel_unblocks_when_child_is_cancelled() {
    let factory = ScenarioFactory::new(17);
    let spec = spec("agent-a", "invocation-a", request(), 2);
    let (sender, receiver) = child_event_channel(&spec);
    let cancellation = CancellationToken::new();
    let child_cancel = cancellation.clone();
    let factory = Arc::new(factory);
    let task_factory = Arc::clone(&factory);
    let task =
        tokio::spawn(
            async move { run_child(spec, task_factory.as_ref(), sender, child_cancel).await },
        );

    assert_eq!(receiver.capacity(), MAX_CHILD_EVENT_QUEUE_CAPACITY);
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while receiver.capacity() > 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(factory.stats.executor_calls.load(Ordering::SeqCst) > 0);
    assert!(!task.is_finished());

    cancellation.cancel_with_reason(CancellationReason::User);
    let result = tokio::time::timeout(std::time::Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        result,
        Err(ChildRunError::Cancelled {
            reason: CancellationReason::User
        })
    ));
}

#[tokio::test]
async fn cancellation_during_final_complete_publish_is_not_success() {
    // The three setup events, one useful metric, 22 text deltas, five attempt
    // metrics, and one tool-count observation fill the 32-slot queue before
    // the final Complete event is published.
    let factory = ScenarioFactory::with_text_deltas(22);
    let child_spec = spec("agent-a", "invocation-a", request(), 2);
    let (sender, mut receiver) = child_event_channel(&child_spec);
    let cancellation = CancellationToken::new();
    let child_cancel = cancellation.clone();
    let task =
        tokio::spawn(async move { run_child(child_spec, &factory, sender, child_cancel).await });

    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while receiver.capacity() > 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    tokio::task::yield_now().await;
    assert!(!task.is_finished());
    receiver.recv().await.expect("the channel is full");
    cancellation.cancel_with_reason(CancellationReason::User);

    let result = tokio::time::timeout(std::time::Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        result,
        Err(ChildRunError::Cancelled {
            reason: CancellationReason::User
        })
    ));
}
