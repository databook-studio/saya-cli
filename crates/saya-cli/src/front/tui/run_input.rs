//! The queue's producers: the terminal's own events, and the tick.
//!
//! Both are turned into messages here and nowhere else, so the loop above them
//! is a function of its queue: a key press, a pasted block and a resize are read
//! on a thread of their own (crossterm's `event::poll` waits by design), and
//! the tick is sent on a period *only while the surface has something that
//! moves* — a running turn or words a toast has still to spend. An idle surface
//! is never woken with a message of its own.
//!
//! The reader can be stopped: `/setup` hands the terminal to the guided wizard,
//! and the wizard must be the only one reading stdin for as long as it owns it.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use ratatui::crossterm::event::{self, Event, KeyEventKind};
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::JoinHandle;

use super::model::Model;
use super::msg::Msg;

/// How long one wait for a terminal event lasts before the reader looks at the
/// gate again. Short enough that a handoff is never held up by more than this,
/// long enough that an idle surface is not a busy-wait — today's pump cadence.
const WAIT: Duration = Duration::from_millis(50);

/// The one message a terminal event means, or none when the surface has nothing
/// to say about it: a focus change and a mouse move are not keys, and a paste's
/// `\r` must never read as Enter.
pub(crate) fn msg_of(event: Event) -> Option<Msg> {
    match event {
        // Windows reports a release for every key as well as the press; a
        // release is not a second keystroke. Repeat stays: a held key must keep
        // repeating on every platform. The rule is platform-independent.
        Event::Key(key) if key.kind != KeyEventKind::Release => Some(Msg::Key(key)),
        Event::Key(_) => None,
        Event::Paste(text) => Some(Msg::Paste(text)),
        Event::Resize(width, height) => Some(Msg::Resize { width, height }),
        // Whether the window has the operator's focus, when the terminal
        // reports it: a long turn's end is only news while they look elsewhere.
        Event::FocusGained => Some(Msg::Focus(true)),
        Event::FocusLost => Some(Msg::Focus(false)),
        _ => None,
    }
}

/// Whether the surface has something that moves: a turn in flight, or words the
/// toast has still to spend. Nothing else on the surface changes on its own, so
/// nothing else earns a tick.
pub(crate) fn animating(model: &Model) -> bool {
    model.is_busy() || model.toast().is_some()
}

/// Whether the surface's reader may read the terminal, and whether it is
/// reading now.
///
/// A handoff (`/setup`) needs both facts: the wizard may not share stdin with
/// the surface, so the reader has to be *out* of a read before the terminal is
/// handed over — not merely asked to stop, because a read in flight would take
/// the wizard's first line. [`InputGate::pause`] therefore waits for the reader
/// to park, and [`InputGate::resume`] lets it read again.
pub(crate) struct InputGate {
    state: Mutex<Gate>,
    wake: Condvar,
}

/// The gate's two facts, behind the one lock.
#[derive(Debug)]
struct Gate {
    /// The reader may read.
    running: bool,
    /// The reader is parked, waiting for permission: it holds no read.
    parked: bool,
}

impl InputGate {
    /// A gate that lets the reader read.
    pub(crate) fn new() -> Self {
        Self {
            state: Mutex::new(Gate {
                running: true,
                parked: false,
            }),
            wake: Condvar::new(),
        }
    }

    /// Waits until the reader may read, and marks it back at work.
    fn enter(&self) {
        let mut gate = self.lock();
        while !gate.running {
            gate.parked = true;
            self.wake.notify_all();
            gate = self.wake.wait(gate).unwrap_or_else(PoisonError::into_inner);
        }
        gate.parked = false;
    }

    /// Stops the reader and waits until it is parked: after this returns, no
    /// read is in flight and everything typed from here on is somebody else's.
    pub(crate) fn pause(&self) {
        let mut gate = self.lock();
        gate.running = false;
        while !gate.parked {
            gate = self.wake.wait(gate).unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// Lets the reader read again: the surface has the terminal back.
    pub(crate) fn resume(&self) {
        let mut gate = self.lock();
        gate.running = true;
        self.wake.notify_all();
    }

    /// The gate, with a poisoned lock read as the value it last held: a
    /// panicking reader must not take the surface's own input with it.
    fn lock(&self) -> MutexGuard<'_, Gate> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Reads the terminal on a thread of its own — one bounded wait per event — and
/// hands each one to the loop. The thread ends with the queue: a loop that is
/// gone takes the reader with it.
pub(crate) fn spawn_input(
    msgs: UnboundedSender<Msg>,
    gate: Arc<InputGate>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        loop {
            // Parked while a handoff owns the terminal, and reading again only
            // once it has given it back.
            gate.enter();
            match event::poll(WAIT) {
                Ok(true) => match event::read() {
                    Ok(event) => {
                        if let Some(msg) = msg_of(event)
                            && msgs.send(msg).is_err()
                        {
                            return;
                        }
                    }
                    // The terminal went away under the reader: the loop finds
                    // out on its own queue ending.
                    Err(_) => return,
                },
                Ok(false) => {}
                Err(_) => return,
            }
        }
    })
}

/// How often the surface looks at the run's job panel: a job's worker reports
/// on its own thread, so its words arrive at a pace of their own.
pub(crate) const JOBS_PERIOD: Duration = Duration::from_millis(25);

/// Looks at the run's job panel on a thread of its own, once per
/// [`JOBS_PERIOD`]: a job's queues are `std::sync` ones and an idle surface
/// paints nothing, so no frame of the loop would ever read them. Ends when the
/// loop's queue closes — the surface is gone, and so is every job it owns.
pub(crate) fn spawn_jobs(front: Arc<super::run_front::Forward>) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        while front.pump() {
            std::thread::sleep(JOBS_PERIOD);
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
