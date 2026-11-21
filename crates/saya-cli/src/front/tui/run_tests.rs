//! The loop's contract: what a frame costs, and what a message does.
//!
//! Every test drives the real `Runtime` over the fake screen, clock and port:
//! the queue is the only way in, so what is asserted here is what an operator
//! with a terminal would see — the number of paints, the last frame's words,
//! and the commands the session's API was asked to perform.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use ratatui::crossterm::event::KeyCode;
use saya_agent::{AgentEvent, PermissionMode};
use tokio::sync::mpsc;

use super::model::Model;
use super::msg::Msg;
use super::run::Runtime;
use super::run_fixture::{FakeScreen, busy, key, toasting, typed};
use super::run_fixture_port::{Call, FakeClock, FakePort};
use crate::agent::tools::consent::Answer;
use crate::chat::{ChatEvent, ChatOutcome, Question};
use crate::front::tui::run_input::animating;

/// A loop over a fresh model, with the record and the port it drives.
fn loop_over(model: Model) -> (Runtime<FakeScreen, FakeClock, FakePort>, Sender, Handle) {
    let (sender, queue) = mpsc::unbounded_channel();
    let screen = FakeScreen::new(100, 30);
    let recorded = screen.recorded();
    let clock = FakeClock::new();
    let port = Arc::new(FakePort::new());
    let runtime = Runtime::new(model, queue, screen, clock.clone(), Arc::clone(&port));
    (
        runtime,
        Sender(sender),
        Handle {
            recorded,
            clock,
            port,
        },
    )
}

/// The queue's writing end.
struct Sender(mpsc::UnboundedSender<Msg>);

impl Sender {
    fn push(&self, msg: Msg) {
        assert!(self.0.send(msg).is_ok(), "the loop is still reading");
    }
}

/// What the test reads back after driving the loop.
struct Handle {
    recorded: Arc<std::sync::Mutex<super::run_fixture::Recorded>>,
    clock: FakeClock,
    port: Arc<FakePort>,
}

impl Handle {
    fn draws(&self) -> usize {
        self.recorded
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .draws()
    }

    fn frame(&self) -> String {
        self.recorded
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .last_frame()
            .to_owned()
    }

    fn calls(&self) -> Vec<Call> {
        self.port.calls()
    }
}

/// One delta of an answer, as the engine streams it.
fn delta(text: &str) -> Msg {
    Msg::Chat(ChatEvent::Agent(AgentEvent::AssistantText {
        text: text.to_owned(),
    }))
}

#[tokio::test]
async fn an_idle_surface_paints_nothing() {
    let (runtime, sender, handle) = loop_over(Model::default());
    let loop_task = tokio::spawn(async move { runtime.run().await });
    // Two seconds of the surface's own clock pass with nothing to fold: the
    // loop is blocked on the queue, so there is no frame and nothing to wake
    // it — an idle surface paints nothing at all.
    handle.clock.advance(Duration::from_secs(2));
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(handle.draws(), 0);
    assert!(!animating(&Model::default()));
    drop(sender);
    loop_task.await.expect("the loop ends").expect("cleanly");
}

#[tokio::test]
async fn a_burst_of_a_hundred_deltas_paints_at_most_thirty_frames() {
    let (mut runtime, sender, handle) = loop_over(Model::default());
    for index in 0..100 {
        let text = if index == 99 { "omega" } else { "a" };
        handle.clock.advance(Duration::from_millis(10));
        sender.push(delta(text));
        runtime.step().await.expect("one turn of the loop");
    }
    // A hundred deltas a tenth of a second apart cover a second of the
    // surface's clock; the limit lets a couple of dozen of them be painted.
    let during = handle.draws();
    assert!(
        during <= 30,
        "a hundred deltas in a second must not paint more than thirty frames, painted {during}"
    );
    // The frame the last deltas changed is owed when the burst stops: it is
    // painted by the next message — the tick a streaming turn sends itself —
    // once its 33 ms are up, and it is the frame that carries the last delta.
    handle.clock.advance(super::run::FRAME);
    sender.push(Msg::Tick);
    runtime.step().await.expect("one turn of the loop");
    assert_eq!(
        handle.draws(),
        during + 1,
        "the owed frame, and only that one"
    );
    assert!(handle.frame().contains("omega"), "{}", handle.frame());
}

#[tokio::test]
async fn a_frame_held_back_by_the_limit_is_painted_when_its_time_comes() {
    let (mut runtime, sender, handle) = loop_over(Model::default());
    sender.push(delta("first"));
    runtime.step().await.expect("one turn of the loop");
    assert_eq!(handle.draws(), 1);
    // Ten milliseconds later the next delta changes the model again, and the
    // frame that would show it is held back rather than painted at once.
    handle.clock.advance(Duration::from_millis(10));
    sender.push(delta("second"));
    runtime.step().await.expect("one turn of the loop");
    assert_eq!(handle.draws(), 1, "the limit holds the frame back");
    // Once the 33 ms are up, the next message paints it.
    handle.clock.advance(Duration::from_millis(25));
    sender.push(Msg::Key(ratatui::crossterm::event::KeyEvent::new(
        KeyCode::Char('x'),
        ratatui::crossterm::event::KeyModifiers::NONE,
    )));
    runtime.step().await.expect("one turn of the loop");
    assert_eq!(handle.draws(), 2, "the held frame was painted");
    assert!(handle.frame().contains("firstsecond"), "{}", handle.frame());
}

#[tokio::test]
async fn enter_on_a_composer_line_sends_exactly_one_message() {
    let (mut runtime, sender, handle) = loop_over(typed("how many orders this month"));
    sender.push(key(KeyCode::Enter));
    runtime.step().await.expect("one turn of the loop");
    assert_eq!(
        handle.calls(),
        vec![Call::Send("how many orders this month".to_owned())]
    );
}

#[tokio::test]
async fn esc_while_a_turn_runs_asks_the_session_to_cancel_it() {
    let (mut runtime, sender, handle) = loop_over(busy());
    sender.push(key(KeyCode::Esc));
    runtime.step().await.expect("one turn of the loop");
    assert_eq!(handle.calls(), vec![Call::Cancel]);
}

#[tokio::test]
async fn a_card_is_answered_through_the_port_and_the_answer_carries_its_id() {
    let mut model = busy();
    model.show_card(Question {
        id: 7,
        caller: String::from("the shell tool"),
        facts: String::from("rm -rf build"),
        grant: None,
    });
    let (mut runtime, sender, handle) = loop_over(model);
    sender.push(key(KeyCode::Char('y')));
    runtime.step().await.expect("one turn of the loop");
    assert_eq!(handle.calls(), vec![Call::Answer(7, Answer::Once)]);
}

#[tokio::test]
async fn shift_tab_states_the_next_mode_through_the_port() {
    let (mut runtime, sender, handle) = loop_over(Model::default());
    sender.push(key(KeyCode::BackTab));
    runtime.step().await.expect("one turn of the loop");
    assert_eq!(
        handle.calls(),
        vec![Call::SwitchMode(PermissionMode::AcceptEdits)]
    );
}

#[tokio::test]
async fn the_ticker_is_only_set_while_something_animates() {
    // Nothing on a quiet surface moves, so nothing asks for a tick.
    assert!(!animating(&Model::default()));
    assert!(animating(&busy()));
    assert!(animating(&toasting()));

    // The same rule through the loop: the flag the ticker reads follows the
    // model the loop has just folded.
    let (mut runtime, sender, _handle) = loop_over(typed("hello"));
    assert!(!runtime.animating_flag().load(Ordering::Relaxed));
    sender.push(key(KeyCode::Enter));
    runtime.step().await.expect("one turn of the loop");
    assert!(runtime.animating_flag().load(Ordering::Relaxed));
    sender.push(Msg::Chat(ChatEvent::TurnEnded(ChatOutcome::Answered)));
    runtime.step().await.expect("one turn of the loop");
    assert!(!runtime.animating_flag().load(Ordering::Relaxed));
}

#[tokio::test]
async fn a_panic_while_folding_restores_the_terminal_and_is_raised() {
    let (sender, queue) = mpsc::unbounded_channel();
    let sender = Sender(sender);
    let screen = FakeScreen::new(80, 24);
    let recorded = screen.recorded();
    let runtime = Runtime::new(
        typed("hello"),
        queue,
        screen,
        FakeClock::new(),
        Arc::new(FakePort::panicking()),
    );
    let loop_task = tokio::spawn(async move { runtime.run().await });
    sender.push(key(KeyCode::Enter));
    // The panic must come out of the loop, not stop it: a loop that swallowed
    // it would leave the surface running on a terminal nobody restored.
    let ended = tokio::time::timeout(Duration::from_secs(5), loop_task)
        .await
        .expect("the panic came out of the loop")
        .expect_err("the panic is not swallowed");
    assert!(ended.is_panic(), "{ended}");
    let recorded = recorded
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    assert!(
        recorded.restored(),
        "the terminal was put back on the way out: {:?}",
        recorded.acts
    );
}
