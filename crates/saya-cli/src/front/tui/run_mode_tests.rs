//! The mode picker through the loop: `/mode` typed into the composer, the
//! keys the pane reads, and the one command the session's API is asked for.
//!
//! Driven the way the runtime drives it — `T14`'s fake screen, clock and port,
//! and nothing but the message queue in — so what is asserted is what an
//! operator would see and what the run would be told.

use std::sync::Arc;
use std::sync::{Mutex, PoisonError};

use ratatui::crossterm::event::KeyCode;
use saya_agent::PermissionMode;
use tokio::sync::mpsc;

use super::model::Model;
use super::msg::Msg;
use super::run::{FRAME, Runtime};
use super::run_fixture::{FakeScreen, Recorded, key};
use super::run_fixture_port::{Call, FakeClock, FakePort};

/// The last frame the fake screen painted.
fn last(recorded: &Arc<Mutex<Recorded>>) -> String {
    recorded
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .last_frame()
        .to_owned()
}

#[tokio::test]
async fn slash_mode_opens_the_picker_and_a_row_states_the_mode_end_to_end() {
    let (sender, queue) = mpsc::unbounded_channel();
    let screen = FakeScreen::new(100, 30);
    let recorded = screen.recorded();
    let clock = FakeClock::new();
    let port = Arc::new(FakePort::new());
    let mut model = Model::default();
    model.insert("/mode");
    let mut runtime = Runtime::new(model, queue, screen, clock.clone(), Arc::clone(&port));

    sender.send(key(KeyCode::Enter)).expect("the loop reads");
    clock.advance(FRAME);
    runtime.step().await.expect("one turn of the loop");

    let painted = last(&recorded);
    assert!(
        painted.contains("╭ Mode"),
        "`/mode` with no argument paints the picker: {painted}"
    );
    assert!(
        painted.contains("read and plan only; writes refuse"),
        "its rows are 02 §8's words: {painted}"
    );
    assert!(
        port.calls().is_empty(),
        "opening the picker tells the session nothing: {:?}",
        port.calls()
    );

    // Two rows down from ask is plan.
    for _ in 0..2 {
        sender.send(key(KeyCode::Down)).expect("the loop reads");
        clock.advance(FRAME);
        runtime.step().await.expect("one turn of the loop");
    }
    sender.send(key(KeyCode::Enter)).expect("the loop reads");
    clock.advance(FRAME);
    runtime.step().await.expect("one turn of the loop");

    assert_eq!(
        port.calls(),
        vec![Call::SwitchMode(PermissionMode::Plan)],
        "the row set goes through the session's one switch, once"
    );
    let painted = last(&recorded);
    assert!(
        !painted.contains("╭ Mode"),
        "the picker closes on Enter: {painted}"
    );
    assert!(
        painted.contains("mode → plan"),
        "and the toast row says what changed: {painted}"
    );
    assert!(
        painted.contains("plan"),
        "the status line names the mode in force: {painted}"
    );
}

#[tokio::test]
async fn esc_over_the_picker_closes_it_without_telling_the_session_anything() {
    let (sender, queue) = mpsc::unbounded_channel();
    let screen = FakeScreen::new(100, 30);
    let recorded = screen.recorded();
    let clock = FakeClock::new();
    let port = Arc::new(FakePort::new());
    let mut model = Model::default();
    model.insert("/mode");
    let mut runtime = Runtime::new(model, queue, screen, clock.clone(), Arc::clone(&port));

    sender.send(key(KeyCode::Enter)).expect("the loop reads");
    clock.advance(FRAME);
    runtime.step().await.expect("one turn of the loop");
    sender.send(key(KeyCode::Down)).expect("the loop reads");
    clock.advance(FRAME);
    runtime.step().await.expect("one turn of the loop");
    sender.send(key(KeyCode::Esc)).expect("the loop reads");
    clock.advance(FRAME);
    runtime.step().await.expect("one turn of the loop");

    assert!(
        port.calls().is_empty(),
        "a cancel states nothing: {:?}",
        port.calls()
    );
    let painted = last(&recorded);
    assert!(!painted.contains("╭ Mode"), "the picker is gone: {painted}");
}

#[tokio::test]
async fn two_shift_tabs_keep_the_welcome_and_say_the_mode_once() {
    let (sender, queue) = mpsc::unbounded_channel();
    let screen = FakeScreen::new(100, 30);
    let recorded = screen.recorded();
    let clock = FakeClock::new();
    let port = Arc::new(FakePort::new());
    let mut runtime = Runtime::new(Model::default(), queue, screen, clock.clone(), port);

    for _ in 0..2 {
        sender
            .send(Msg::Key(ratatui::crossterm::event::KeyEvent::new(
                KeyCode::BackTab,
                ratatui::crossterm::event::KeyModifiers::SHIFT,
            )))
            .expect("the loop reads");
        clock.advance(FRAME);
        runtime.step().await.expect("one turn of the loop");
    }

    let painted = last(&recorded);
    assert_eq!(
        painted.matches("mode: ").count(),
        0,
        "no turn was begun, so no notice is written into one: {painted}"
    );
    assert!(
        painted.contains("Describe a change") || painted.contains("/connect"),
        "the welcome is still the screen: {painted}"
    );
    assert!(
        painted.contains("mode → plan"),
        "the toast row says the last mode: {painted}"
    );
}
