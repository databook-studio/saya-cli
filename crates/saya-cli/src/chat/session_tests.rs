//! The session's own life, driven by a fake run port: preparing, one turn from
//! `Ready` back to `Ready`, and a prepare that fails without losing the
//! operator's next message.

use super::support::{FakePort, prepared, send_in_background, session};
use super::{Admission, SessionState};

#[tokio::test]
async fn a_prepared_run_opens_the_session() {
    let port = FakePort::new([prepared()]);
    let (session, front) = session(&port);
    assert_eq!(
        session.state(),
        SessionState::Preparing,
        "a new session prepares its run before it takes a message"
    );

    session.open().await;

    assert_eq!(session.state(), SessionState::Ready);
    assert_eq!(
        front.events(),
        vec!["ready db=Some(\"analytics\") workspace=None".to_owned()],
        "the front is told the run is ready, with the profile it connected"
    );
}

#[tokio::test]
async fn a_message_in_ready_runs_one_turn_and_returns_to_ready() {
    let port = FakePort::new([prepared()]);
    port.hold_turns();
    let (session, front) = session(&port);
    session.open().await;

    let running = send_in_background(&session, "hello");
    port.wait_started().await;
    assert_eq!(
        session.state(),
        SessionState::Busy,
        "a turn in flight is the busy state"
    );
    port.release();

    assert_eq!(
        running.await.expect("the send task finishes"),
        Admission::Ran
    );
    assert_eq!(session.state(), SessionState::Ready);
    assert_eq!(port.prompts(), vec!["hello".to_owned()]);
    assert_eq!(front.turn_ends(), vec!["turn-ended Answered".to_owned()]);
}

#[tokio::test]
async fn a_failed_prepare_keeps_the_message_and_runs_it_after_a_retry() {
    let port = FakePort::new([Err("no provider is configured".to_owned()), prepared()]);
    let (session, front) = session(&port);

    session.open().await;
    assert_eq!(session.state(), SessionState::Degraded);
    let notices = front.notices();
    assert_eq!(
        notices.len(),
        1,
        "the failed prepare is said once: {notices:?}"
    );
    assert!(
        notices[0].contains("no provider is configured"),
        "the engine's own words reach the front: {notices:?}"
    );

    assert_eq!(
        session.send("hello".to_owned()).await,
        Admission::Ran,
        "the message is kept and runs once the run is prepared again"
    );

    assert_eq!(
        port.prepares_run(),
        2,
        "the message retries the prepare before it is sent"
    );
    assert_eq!(
        port.prompts(),
        vec!["hello".to_owned()],
        "the message the operator sent is never lost"
    );
    assert_eq!(session.state(), SessionState::Ready);
    let events = front.events();
    assert_eq!(
        events.len(),
        3,
        "the retained message waits for the run, then runs on it: {events:?}"
    );
    assert!(
        events[0].starts_with("notice: ") && events[0].contains("no provider is configured"),
        "the failed prepare is shown first, in the engine's words: {events:?}"
    );
    assert_eq!(
        events[1], "ready db=Some(\"analytics\") workspace=None",
        "the retried prepare opens the run"
    );
    assert_eq!(
        events[2], "turn-ended Answered",
        "the kept message is the turn that runs"
    );
}
