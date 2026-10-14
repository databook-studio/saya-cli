use super::{
    child_scheduler::{caps, finish, ledger, node, start},
    fixture::Fixture,
    openai_server::{ANSWER_RESPONSE, LocalOpenAi},
};
use saya_harness::coding::coordinator::{acknowledge_completion, replay_completions};
use saya_types::CodingResultOutcome;

#[tokio::test]
async fn actual_joined_answer_is_durable_before_parent_ack() {
    let fixture = Fixture::new("r10-actual-joined").await;
    let ledger = ledger(&fixture).await;
    let mut server = LocalOpenAi::new().await;
    let child = node(
        &fixture,
        &server,
        "agent-a",
        "src",
        vec![],
        "provider",
        Default::default(),
    )
    .await;
    let running = start(vec![child], caps(), ledger.clone());
    assert_eq!(server.request().await, Some(1));
    server.respond_sse(ANSWER_RESPONSE);
    let report = finish(running).await;
    assert!(report.stop.is_none());
    let answer = &report.outcomes[0].result.as_ref().unwrap().answer;
    let sequence = report.outcomes[0].completion_sequence.unwrap();
    let replay = replay_completions(&fixture.store, &fixture.job, None, 32)
        .await
        .unwrap();
    assert_eq!(replay.items.len(), 1);
    assert_eq!(replay.items[0].sequence, sequence);
    assert_eq!(replay.items[0].result.answer(), answer);
    assert_eq!(
        replay.items[0].result.outcome(),
        CodingResultOutcome::Completed
    );
    assert!(
        acknowledge_completion(&fixture.store, &fixture.job, sequence)
            .await
            .unwrap()
    );
    assert!(
        !acknowledge_completion(&fixture.store, &fixture.job, sequence)
            .await
            .unwrap()
    );
    assert!(
        replay_completions(&fixture.store, &fixture.job, None, 32)
            .await
            .unwrap()
            .items
            .is_empty()
    );
    server.shutdown().await;
    fixture.close().await;
}
