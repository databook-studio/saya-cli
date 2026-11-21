//! What a busy session does with the operator's messages: one waits (depth one),
//! the next replaces it and says so, the engine's own `Busy` never reaches the
//! front, and a quit waits for the running turn to be cancelled.

use super::support::{ADMIT, FakePort, prepared, send_in_background, session};
use super::{Admission, SessionState};

#[tokio::test]
async fn a_message_sent_while_busy_waits_and_runs_after_the_turn() {
    let port = FakePort::new([prepared()]);
    port.hold_turns();
    let (session, front) = session(&port);
    session.open().await;

    let running = send_in_background(&session, "first");
    port.wait_started().await;

    let second = tokio::time::timeout(ADMIT, session.send("second".to_owned()))
        .await
        .expect("a send while a turn runs is admitted at once, never waiting on the turn");
    assert_eq!(
        second,
        Admission::Queued,
        "a message sent mid-turn waits for the turn to end"
    );
    assert!(
        front.notices().is_empty(),
        "the first waiting message says nothing: only a replacement is reported"
    );
    assert_eq!(
        session.state(),
        SessionState::Busy,
        "queueing a message does not end the turn"
    );

    port.release();
    port.wait_started().await;
    port.release();

    assert_eq!(
        running.await.expect("the send task finishes"),
        Admission::Ran
    );
    assert_eq!(session.state(), SessionState::Ready);
    assert_eq!(
        port.prompts(),
        vec!["first".to_owned(), "second".to_owned()],
        "the waiting message runs when the turn ends, in the order it was sent"
    );
}

#[tokio::test]
async fn a_second_waiting_message_replaces_the_first_and_says_so_once() {
    let port = FakePort::new([prepared()]);
    port.hold_turns();
    let (session, front) = session(&port);
    session.open().await;

    let running = send_in_background(&session, "first");
    port.wait_started().await;

    let second = tokio::time::timeout(ADMIT, session.send("second".to_owned()))
        .await
        .expect("a send while a turn runs is admitted at once");
    assert_eq!(second, Admission::Queued);
    let third = tokio::time::timeout(ADMIT, session.send("third".to_owned()))
        .await
        .expect("a send while one message waits is admitted at once");
    assert_eq!(third, Admission::Queued);

    let notices = front.notices();
    assert_eq!(
        notices.len(),
        1,
        "replacing a waiting message is reported once, never silently: {notices:?}"
    );
    assert!(
        notices[0].contains("replaced"),
        "the notice says the waiting message was replaced: {notices:?}"
    );

    port.release();
    port.wait_started().await;
    port.release();

    running.await.expect("the send task finishes");
    assert_eq!(
        port.prompts(),
        vec!["first".to_owned(), "third".to_owned()],
        "the queue is one deep: the operator's last word is the one that runs"
    );
}

#[tokio::test]
async fn a_port_answering_busy_is_never_shown() {
    let port = FakePort::new([prepared()]);
    port.busy(1);
    let (session, front) = session(&port);
    session.open().await;

    assert_eq!(session.send("hello".to_owned()).await, Admission::Ran);

    assert_eq!(
        port.calls(),
        2,
        "the message is offered again instead of being handed back"
    );
    assert_eq!(
        port.prompts(),
        vec!["hello".to_owned()],
        "the message ran exactly once"
    );
    assert_eq!(session.state(), SessionState::Ready);
    assert_eq!(front.turn_ends(), vec!["turn-ended Answered".to_owned()]);
    assert!(
        !front.events().iter().any(|event| event.contains("busy")),
        "busy is the engine's bookkeeping and never reaches the front: {:?}",
        front.events()
    );
}

#[tokio::test]
async fn quitting_while_busy_is_refused_until_the_turn_is_cancelled() {
    let port = FakePort::new([prepared()]);
    port.hold_turns();
    let (session, front) = session(&port);
    session.open().await;

    let running = send_in_background(&session, "hello");
    port.wait_started().await;

    let refusal = session
        .quit()
        .await
        .expect_err("a quit during a turn is refused, not obeyed");
    assert!(
        refusal.contains("cancel"),
        "the refusal tells the operator to cancel the turn first: {refusal}"
    );
    assert_eq!(
        session.state(),
        SessionState::Busy,
        "a refused quit leaves the turn running"
    );

    session.cancel().await;
    assert_eq!(
        running.await.expect("the send task finishes"),
        Admission::Ran,
        "a cancelled turn still reports its admission"
    );
    assert_eq!(
        front.turn_ends(),
        vec!["turn-ended Cancelled".to_owned()],
        "the turn the operator cancelled ends as cancelled"
    );

    session.quit().await.expect("a quiet session closes");
    assert_eq!(session.state(), SessionState::Closed);
    assert!(port.closed(), "closing the session closes the run");
}
