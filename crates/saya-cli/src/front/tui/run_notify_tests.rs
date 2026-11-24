//! Notifications (UI8): the bell and the desktop sequence when a call takes
//! the card, and when a turn longer than the configured threshold ends.
//!
//! The runtime is driven through its queue exactly as a terminal would drive
//! it, so what is asserted is what the screen was asked to write.

use std::sync::Arc;
use std::time::Duration;

use ratatui::crossterm::event::KeyCode;
use tokio::sync::mpsc;

use super::model::Model;
use super::msg::Msg;
use super::notify::Notify;
use super::run::Runtime;
use super::run_fixture::{FakeScreen, Recorded, key, typed};
use super::run_fixture_port::{FakeClock, FakePort};
use super::update::update;
use super::update_fixture::escape;
use super::view::cards_fixture::shell_question;
use crate::chat::{ChatEvent, ChatOutcome};
use crate::front::tui::run_fixture::Act;

/// One runtime over a recording screen and a hand-turned clock.
struct Harness {
    runtime: Runtime<FakeScreen, FakeClock, FakePort>,
    sender: mpsc::UnboundedSender<Msg>,
    recorded: Arc<std::sync::Mutex<Recorded>>,
    clock: FakeClock,
}

impl Harness {
    fn over(model: Model) -> Self {
        let (sender, queue) = mpsc::unbounded_channel();
        let screen = FakeScreen::new(100, 30);
        let recorded = screen.recorded();
        let clock = FakeClock::new();
        let runtime = Runtime::new(
            model,
            queue,
            screen,
            clock.clone(),
            Arc::new(FakePort::new()),
        );
        Self {
            runtime,
            sender,
            recorded,
            clock,
        }
    }

    fn push(&self, msg: Msg) {
        assert!(self.sender.send(msg).is_ok(), "the loop is still reading");
    }

    async fn step(&mut self) {
        self.runtime.step().await.expect("one turn of the loop");
    }

    /// How many notifications the screen was asked to write.
    fn notifies(&self) -> usize {
        self.recorded
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .acts
            .iter()
            .filter(|act| **act == Act::Notify)
            .count()
    }

    /// Whether the old bell-only act was ever written.
    fn bells(&self) -> usize {
        self.recorded
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .acts
            .iter()
            .filter(|act| **act == Act::Bell)
            .count()
    }
}

/// A call that takes the card: the screen is told, once.
#[tokio::test]
async fn an_approval_notifies_the_terminal_once() {
    let mut harness = Harness::over(Model::default());
    harness.push(Msg::Chat(ChatEvent::Question(shell_question())));
    harness.step().await;

    assert_eq!(
        harness.notifies(),
        1,
        "the approval wrote the bell and the desktop sequence once"
    );
}

/// A turn past the threshold: its end is news.
#[tokio::test]
async fn a_long_turn_notifies_when_it_ends() {
    let mut harness = Harness::over(typed("hello"));
    harness.push(key(KeyCode::Enter));
    harness.step().await;
    harness.clock.advance(Duration::from_secs(31));
    harness.push(Msg::Chat(ChatEvent::TurnEnded(ChatOutcome::Answered)));
    harness.step().await;

    assert_eq!(
        harness.notifies(),
        1,
        "a turn longer than thirty seconds notifies when it ends"
    );
}

/// A short turn is not news.
#[tokio::test]
async fn a_short_turn_does_not_notify() {
    let mut harness = Harness::over(typed("hello"));
    harness.push(key(KeyCode::Enter));
    harness.step().await;
    harness.clock.advance(Duration::from_secs(2));
    harness.push(Msg::Chat(ChatEvent::TurnEnded(ChatOutcome::Answered)));
    harness.step().await;

    assert_eq!(harness.notifies(), 0, "two seconds is not a long turn");
}

/// The terminal says it has the focus: a long turn's end is not news.
#[tokio::test]
async fn a_long_turn_does_not_notify_while_the_terminal_is_focused() {
    let mut harness = Harness::over(typed("hello"));
    harness.push(Msg::Focus(true));
    harness.push(key(KeyCode::Enter));
    harness.step().await;
    harness.clock.advance(Duration::from_secs(31));
    harness.push(Msg::Chat(ChatEvent::TurnEnded(ChatOutcome::Answered)));
    harness.step().await;

    assert_eq!(
        harness.notifies(),
        0,
        "the operator was already looking at the turn"
    );
}

/// The switch: off means no bell, no desktop sequence, and no old bell either.
#[tokio::test]
async fn notifications_off_means_none_at_all() {
    let off = || Model {
        notify: Some(Notify {
            enabled: false,
            ..Notify::default()
        }),
        ..Model::default()
    };
    let mut model = off();
    let cmds = update(&mut model, Msg::Chat(ChatEvent::Question(shell_question())));
    assert!(
        !cmds.contains(&super::msg::Cmd::Bell),
        "the approval does not ring when notifications are off: {cmds:?}"
    );

    let mut harness = Harness::over(off());
    harness.push(Msg::Chat(ChatEvent::Question(shell_question())));
    harness.step().await;
    assert_eq!(harness.notifies(), 0);
    assert_eq!(harness.bells(), 0, "the old bell-only write is gone too");

    // And a long turn stays silent.
    let mut harness = Harness::over(Model {
        notify: Some(Notify {
            enabled: false,
            ..Notify::default()
        }),
        ..typed("hello")
    });
    harness.push(key(KeyCode::Enter));
    harness.step().await;
    harness.clock.advance(Duration::from_secs(90));
    harness.push(Msg::Chat(ChatEvent::TurnEnded(ChatOutcome::Answered)));
    harness.step().await;
    assert_eq!(harness.notifies(), 0);
}

/// The configured threshold is the one that holds.
#[tokio::test]
async fn the_configured_threshold_is_the_one_that_holds() {
    let mut harness = Harness::over(Model {
        notify: Some(Notify {
            after: Duration::from_secs(5),
            ..Notify::default()
        }),
        ..typed("hello")
    });
    harness.push(key(KeyCode::Enter));
    harness.step().await;
    harness.clock.advance(Duration::from_secs(6));
    harness.push(Msg::Chat(ChatEvent::TurnEnded(ChatOutcome::Answered)));
    harness.step().await;

    assert_eq!(
        harness.notifies(),
        1,
        "six seconds passes a five-second threshold"
    );
}

/// The Esc edit still runs; a card's Esc is still a denial, not a notification
/// decision. This pins that the notification path never rings for a key.
#[tokio::test]
async fn a_plain_key_does_not_notify() {
    let mut harness = Harness::over(Model::default());
    harness.push(escape());
    harness.step().await;
    assert_eq!(harness.notifies(), 0);
}

/// What the terminal is written: the bell, then the desktop sequence a terminal
/// that knows it reads, and nothing else.
#[test]
fn the_notification_write_is_one_bell_then_one_desktop_sequence() {
    let mut out = Vec::new();
    super::run_screen::write_attention(&mut out).expect("a vector writes");

    assert_eq!(
        out, b"\x07\x1b]9;Saya needs attention\x07",
        "one bell and one OSC 9 sequence, in that order"
    );
    assert_eq!(
        out.iter().filter(|byte| **byte == 0x07).count(),
        2,
        "the bell and the sequence's terminator"
    );
}
