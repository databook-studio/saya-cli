use super::fixture::*;
use saya_store::CodingApprovalConsumptionAppend;
use saya_types::{CodingEventKind, JobState};

#[tokio::test]
async fn root_and_current_child_consumption_write_one_consecutive_pair() {
    let (path, store, job) = running("owner-pairs").await;
    let root_request = append_request(
        &store,
        &job,
        2,
        None,
        now().saturating_sub(1),
        now() + 60_000,
    )
    .await;
    assert!(matches!(
        store
            .consume_coding_approval(
                reservation(&job, 3, None, 1),
                closure(&job, 2, 3, None),
                context(&root_request)
            )
            .await
            .unwrap(),
        CodingApprovalConsumptionAppend::Recorded(_)
    ));
    let actor = child(&store, &job, "current").await;
    let child_request = request(
        &job,
        5,
        Some((&actor.0, &actor.1)),
        RequestSpec {
            digest: "a",
            epoch: 7,
            requested: now().saturating_sub(1),
            expires: now() + 60_000,
            host: "example.com",
        },
    );
    store
        .record_coding_approval_request(request_event(&child_request))
        .await
        .unwrap();
    assert!(matches!(
        store
            .consume_coding_approval(
                reservation(&job, 6, Some((&actor.0, &actor.1)), 1),
                closure(&job, 5, 6, Some((&actor.0, &actor.1))),
                context(&child_request)
            )
            .await
            .unwrap(),
        CodingApprovalConsumptionAppend::Recorded(_)
    ));
    let child_read = store
        .get_coding_approval_request(&job, child_request.request_sequence())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        child_read.closure.unwrap().agent_id.as_ref(),
        Some(&actor.0)
    );
    store.close().await;
    let _ = std::fs::remove_dir_all(path);
}

#[tokio::test]
async fn exact_consumption_retry_after_expiry_and_terminal_state_is_audit_only() {
    let (path, store, job) = running("expired-duplicate").await;
    let now = now();
    let req = append_request(&store, &job, 2, None, now.saturating_sub(1), now + 800).await;
    let reservation_event = reservation(&job, 3, None, 1);
    let closure_event = closure(&job, 2, 3, None);
    let context = context(&req);
    assert!(matches!(
        store
            .consume_coding_approval(
                reservation_event.clone(),
                closure_event.clone(),
                context.clone()
            )
            .await
            .unwrap(),
        CodingApprovalConsumptionAppend::Recorded(_)
    ));
    store
        .append_coding_event(event(
            5,
            &job,
            None,
            CodingEventKind::StateChanged {
                state: JobState::Completed,
            },
        ))
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(900)).await;
    assert!(matches!(
        store
            .consume_coding_approval(reservation_event, closure_event, context)
            .await
            .unwrap(),
        CodingApprovalConsumptionAppend::AlreadyPresent(_)
    ));
    assert_eq!(
        store.get_coding_job(&job).await.unwrap().unwrap().state,
        JobState::Completed
    );
    store.close().await;
    let _ = std::fs::remove_dir_all(path);
}
