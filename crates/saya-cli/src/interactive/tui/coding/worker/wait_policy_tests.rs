//! How a question waits for the operator's answer. A detached wait gives up at
//! its deadline, so a panel nobody is watching cannot hold a run open; an
//! attached wait ends only on the answer or on cancellation. Time is paused in
//! every test here, so no test really waits: the clock is advanced instead.

use super::super::CodingMessage;
use super::{WaitPolicy, ask_operator};
use saya_agent::CancellationToken;
use std::sync::mpsc;
use std::time::Duration;

/// Both policies, for the outcomes that must refuse under either one.
fn policies() -> [WaitPolicy; 2] {
    [WaitPolicy::Attached, WaitPolicy::PANEL]
}

/// One question in flight: the worker's pending answer, its queue and the run's
/// token, so a test can answer, drain or cancel it.
struct Asked {
    answered: tokio::task::JoinHandle<bool>,
    queue: mpsc::Receiver<CodingMessage>,
    token: CancellationToken,
}

fn ask(policy: WaitPolicy) -> Asked {
    let (sender, queue) = mpsc::sync_channel::<CodingMessage>(4);
    let token = CancellationToken::new();
    let asking = token.clone();
    let answered = tokio::spawn(async move {
        ask_operator(&sender, &asking, "Approve job job-a?".into(), policy).await
    });
    Asked {
        answered,
        queue,
        token,
    }
}

/// The queued question, without needing a frame to drain it.
async fn next_message(queue: &mpsc::Receiver<CodingMessage>) -> CodingMessage {
    loop {
        match queue.try_recv() {
            Ok(message) => return message,
            Err(mpsc::TryRecvError::Empty) => tokio::task::yield_now().await,
            Err(mpsc::TryRecvError::Disconnected) => panic!("the wait dropped its queue"),
        }
    }
}

/// The channel the operator's answer travels on.
async fn responder(asked: &Asked) -> tokio::sync::oneshot::Sender<bool> {
    let CodingMessage::Approval { respond, .. } = next_message(&asked.queue).await else {
        panic!("the operator is asked through the approval message");
    };
    respond
}

#[tokio::test(start_paused = true)]
async fn a_detached_wait_refuses_an_unanswered_question_at_its_deadline() {
    let asked = ask(WaitPolicy::PANEL);
    let _responder = responder(&asked).await;
    tokio::time::advance(Duration::from_secs(119)).await;
    assert!(
        !asked.answered.is_finished(),
        "the panel's deadline is 120 s, not 119"
    );
    tokio::time::advance(Duration::from_secs(1)).await;
    assert!(
        !asked.answered.await.unwrap(),
        "an unanswered question is a refusal"
    );
}

#[tokio::test(start_paused = true)]
async fn a_detached_wait_accepts_an_answer_before_its_deadline() {
    let asked = ask(WaitPolicy::PANEL);
    let responder = responder(&asked).await;
    tokio::time::advance(Duration::from_secs(119)).await;
    responder.send(true).expect("the wait is still listening");
    assert!(asked.answered.await.unwrap());
}

#[tokio::test(start_paused = true)]
async fn an_attached_wait_never_gives_up_on_the_person() {
    let asked = ask(WaitPolicy::Attached);
    let responder = responder(&asked).await;
    tokio::time::advance(Duration::from_secs(3_600)).await;
    assert!(
        !asked.answered.is_finished(),
        "an attached wait has no deadline"
    );
    responder.send(true).expect("the wait is still listening");
    assert!(asked.answered.await.unwrap());
}

#[tokio::test(start_paused = true)]
async fn an_attached_wait_is_still_cancelled_while_pending() {
    let asked = ask(WaitPolicy::Attached);
    let _responder = responder(&asked).await;
    asked.token.cancel();
    assert!(
        !asked.answered.await.unwrap(),
        "cancellation is a refusal, not an approval"
    );
}

#[tokio::test(start_paused = true)]
async fn a_dropped_responder_is_refused_under_either_policy() {
    for policy in policies() {
        let asked = ask(policy);
        let responder = responder(&asked).await;
        drop(responder);
        assert!(
            !asked.answered.await.unwrap(),
            "a closed answer channel is a refusal"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn an_answer_of_false_is_refused_under_either_policy() {
    for policy in policies() {
        let asked = ask(policy);
        let responder = responder(&asked).await;
        responder.send(false).expect("the wait is still listening");
        assert!(
            !asked.answered.await.unwrap(),
            "an explicit denial is a refusal"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn a_full_queue_is_refused_without_waiting() {
    for policy in policies() {
        let (sender, queue) = mpsc::sync_channel::<CodingMessage>(1);
        sender
            .try_send(CodingMessage::Activity("progress".into()))
            .expect("room for one event");
        let token = CancellationToken::new();
        let started = tokio::time::Instant::now();
        assert!(
            !ask_operator(&sender, &token, "Approve job job-a?".into(), policy).await,
            "a question that cannot be queued is a refusal"
        );
        assert_eq!(
            tokio::time::Instant::now(),
            started,
            "a full queue is refused before any wait begins"
        );
        assert!(matches!(
            queue.try_recv(),
            Ok(CodingMessage::Activity(text)) if text == "progress"
        ));
    }
}
