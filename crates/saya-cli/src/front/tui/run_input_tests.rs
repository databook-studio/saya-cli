//! The producers: every terminal event the surface cares about becomes exactly
//! one message, and the tick only exists while something moves.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use tokio::sync::mpsc;

use super::model::Model;
use super::msg::Msg;
use super::run_fixture::{busy, toasting};
use super::run_input::{animating, msg_of, spawn_ticker};

#[test]
fn keys_pastes_and_resizes_become_the_messages_they_mean() {
    let press = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE);
    assert!(matches!(
        msg_of(Event::Key(press)),
        Some(Msg::Key(seen)) if seen == press
    ));
    // A pasted block is text and only text: its `\r` must never read as Enter.
    assert!(matches!(
        msg_of(Event::Paste(String::from("select 1;\r"))),
        Some(Msg::Paste(text)) if text == "select 1;\r"
    ));
    assert!(matches!(
        msg_of(Event::Resize(80, 24)),
        Some(Msg::Resize {
            width: 80,
            height: 24
        })
    ));
}

#[test]
fn nothing_else_on_the_terminal_is_a_message() {
    // A focus change is not a key press, and the surface has nothing to do
    // about it: no message, no frame.
    assert!(msg_of(Event::FocusGained).is_none());
    assert!(msg_of(Event::FocusLost).is_none());
}

#[test]
fn the_surface_animates_only_over_a_running_turn_or_a_live_toast() {
    assert!(!animating(&Model::default()));
    assert!(animating(&busy()));
    assert!(animating(&toasting()));
}

#[tokio::test]
async fn an_idle_ticker_sends_nothing_and_an_animating_one_ticks() {
    let (msgs, mut queue) = mpsc::unbounded_channel();
    let animating = Arc::new(AtomicBool::new(false));
    let period = Duration::from_millis(2);
    let ticker = spawn_ticker(Arc::clone(&animating), msgs, period);

    // Nothing moves: the period passes several times over and the queue stays
    // empty.
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(queue.try_recv().is_err(), "an idle surface is never ticked");

    // A turn starts: the tick arrives on the same period.
    animating.store(true, Ordering::Relaxed);
    let tick = tokio::time::timeout(Duration::from_millis(500), queue.recv()).await;
    assert!(matches!(tick, Ok(Some(Msg::Tick))), "a busy surface ticks");
    ticker.abort();
}
