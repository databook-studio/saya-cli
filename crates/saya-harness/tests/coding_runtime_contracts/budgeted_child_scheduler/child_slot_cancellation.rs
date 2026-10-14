use super::{
    child_scheduler::{caps, finish, ledger, node, start},
    fixture::Fixture,
    openai_server::{ANSWER_RESPONSE, LocalOpenAi},
};
use saya_agent::{CancellationReason, MAX_STEERING_MESSAGES, SteeringError};
use saya_harness::coding::coordinator::{ChildRunError, ScheduleChildError};

#[tokio::test]
async fn full_mailbox_does_not_block_individual_cancel_or_sibling() {
    let fixture = Fixture::new("r09-mailbox-cancel").await;
    let ledger = ledger(&fixture).await;
    let mut a = LocalOpenAi::new().await;
    let mut b = LocalOpenAi::new().await;
    let first = node(
        &fixture,
        &a,
        "agent-a",
        "src",
        vec![],
        "provider-a",
        Default::default(),
    )
    .await;
    let handle = first.control();
    let second = node(
        &fixture,
        &b,
        "agent-b",
        "docs",
        vec![],
        "provider-b",
        Default::default(),
    )
    .await;
    let running = start(vec![first, second], caps(), ledger.clone());
    assert_eq!(a.request().await, Some(1));
    assert_eq!(b.request().await, Some(1));
    for _ in 0..MAX_STEERING_MESSAGES {
        handle.send("steer after current turn").unwrap();
    }
    assert_eq!(handle.send("overflow"), Err(SteeringError::Full));
    handle.cancel();
    b.respond_sse(ANSWER_RESPONSE);
    let report = finish(running).await;
    assert!(report.stop.is_none());
    assert!(
        report
            .outcomes
            .iter()
            .any(|item| item.agent_id.as_str() == "agent-b" && item.result.is_ok())
    );
    assert!(report.outcomes.iter().any(|item| matches!(
        &item.result,
        Err(ScheduleChildError::Run(ChildRunError::Cancelled {
            reason: CancellationReason::User
        }))
    )));
    assert_eq!(ledger.snapshot().await.child_slots_active, 0);
    a.shutdown().await;
    b.shutdown().await;
    fixture.close().await;
}
