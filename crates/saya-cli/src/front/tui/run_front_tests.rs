//! The session's front: everything the session says is one message, and the
//! one thing a queue cannot carry — an answer — comes back to the ask that is
//! waiting for it.

use std::sync::Arc;

use saya_agent::AgentEvent;
use tokio::sync::mpsc;

use super::msg::Msg;
use super::run_front::Forward;
use crate::agent::tools::consent::Answer;
use crate::chat::{ChatEvent, ChatFront, Question};

/// A front over a queue nobody else writes to.
fn front() -> (Arc<Forward>, mpsc::UnboundedReceiver<Msg>) {
    let (msgs, queue) = mpsc::unbounded_channel();
    (Arc::new(Forward::new(msgs)), queue)
}

/// One call put to the operator.
fn question(id: u64) -> Question {
    Question {
        id,
        caller: String::from("the shell tool"),
        facts: String::from("rm -rf build"),
        grant: None,
    }
}

#[tokio::test]
async fn a_session_event_reaches_the_queue_as_a_chat_message() {
    let (front, mut queue) = front();
    front.emit(ChatEvent::Notice {
        message: String::from("the run could not be prepared"),
    });
    assert!(matches!(
        queue.try_recv(),
        Ok(Msg::Chat(ChatEvent::Notice { message }))
            if message == "the run could not be prepared"
    ));
}

#[tokio::test]
async fn the_engines_own_stream_reaches_the_same_queue() {
    let (front, mut queue) = front();
    front
        .sink()
        .emit(AgentEvent::AssistantText {
            text: String::from("110 orders"),
        })
        .await;
    assert!(matches!(
        queue.try_recv(),
        Ok(Msg::Chat(ChatEvent::Agent(AgentEvent::AssistantText { text })))
            if text == "110 orders"
    ));
}

#[tokio::test]
async fn a_question_is_put_on_the_queue_and_resumes_with_the_answer_it_is_given() {
    let (front, mut queue) = front();
    let asking = {
        let front = Arc::clone(&front);
        tokio::spawn(async move { front.ask(&question(4)).await })
    };
    // The card is on the queue, waiting for a press.
    let put = queue.recv().await;
    assert!(matches!(put, Some(Msg::Chat(ChatEvent::Question(asked))) if asked.id == 4));
    assert!(front.answer(4, Answer::Session));
    assert_eq!(asking.await.expect("the ask finishes"), Answer::Session);
}

#[tokio::test]
async fn an_answer_to_a_call_nobody_asked_is_refused_rather_than_kept() {
    let (front, _queue) = front();
    assert!(!front.answer(11, Answer::Once));
}

#[tokio::test]
async fn the_surfaces_own_words_reach_the_queue_as_a_workers_word() {
    let (front, mut queue) = front();
    front.say("the mode stays where it is");
    assert!(matches!(
        queue.try_recv(),
        Ok(Msg::Worker(worker)) if worker.message == "the mode stays where it is"
    ));
    // Nothing said is nothing to show.
    front.say("");
    assert!(queue.try_recv().is_err());
}

/// The old surface's pin — it states "never reads stdin, always has its
/// modal" — moves here with the surface. `chat/compose.rs` reads the second
/// half to decide whether a write-shaped tool is advertised at all, so a
/// front that answered the default `false` would hide every tool whose call
/// it can actually ask about. The first half is the surface's shape: the
/// composer reads keys, nothing on this surface reads stdin.
#[tokio::test]
async fn the_tui_front_can_obtain_a_per_call_approval() {
    let (front, _queue) = front();
    assert!(
        front.can_obtain_approval(),
        "the TUI answers calls through its own card, so it can obtain a \
         per-call approval; answering `false` hides every write-shaped tool"
    );
}
