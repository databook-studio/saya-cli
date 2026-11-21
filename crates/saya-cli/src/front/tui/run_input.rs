//! The queue's producers: the terminal's own events, and the tick.
//!
//! Both are turned into messages here and nowhere else, so the loop above them
//! is a function of its queue: a key press, a pasted block and a resize are read
//! on a thread of their own (crossterm's `event::read` blocks by design), and
//! the tick is sent on a period *only while the surface has something that
//! moves* — a running turn or words a toast has still to spend. An idle surface
//! is never woken with a message of its own.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use ratatui::crossterm::event::{self, Event};
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::JoinHandle;

use super::model::Model;
use super::msg::Msg;

/// The one message a terminal event means, or none when the surface has nothing
/// to say about it: a focus change and a mouse move are not keys, and a paste's
/// `\r` must never read as Enter.
pub(crate) fn msg_of(event: Event) -> Option<Msg> {
    match event {
        Event::Key(key) => Some(Msg::Key(key)),
        Event::Paste(text) => Some(Msg::Paste(text)),
        Event::Resize(width, height) => Some(Msg::Resize { width, height }),
        _ => None,
    }
}

/// Whether the surface has something that moves: a turn in flight, or words the
/// toast has still to spend. Nothing else on the surface changes on its own, so
/// nothing else earns a tick.
pub(crate) fn animating(model: &Model) -> bool {
    model.is_busy() || model.toast().is_some()
}

/// Reads the terminal on a thread of its own — one blocking read per event — and
/// hands each one to the loop. The thread ends with the queue: a loop that is
/// gone takes the reader with it.
pub(crate) fn spawn_input(msgs: UnboundedSender<Msg>) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        while let Ok(event) = event::read() {
            if let Some(msg) = msg_of(event)
                && msgs.send(msg).is_err()
            {
                return;
            }
        }
    })
}

/// Sends one [`Msg::Tick`] per `period` while `animating` is set, and nothing at
/// all while it is not.
///
/// The ticker wakes on its own period either way; what the flag guards is the
/// *message*. A surface with nothing to animate therefore leaves the loop
/// blocked on the queue, which is the rule the render loop is built on.
pub(crate) fn spawn_ticker(
    animating: Arc<AtomicBool>,
    msgs: UnboundedSender<Msg>,
    period: Duration,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(period).await;
            if animating.load(Ordering::Relaxed) && msgs.send(Msg::Tick).is_err() {
                return;
            }
        }
    })
}
