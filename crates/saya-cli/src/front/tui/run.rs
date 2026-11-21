//! The runtime: one queue in, at most one frame out per change.
//!
//! `04` §10's loop, and nothing else. It blocks on the message queue — there is
//! no polling of its own — folds every message that is already waiting through
//! the pure [`update`](super::update), performs the commands that update
//! answered with, and paints a frame only when the model changed *and* the last
//! frame is at least [`FRAME`] old. An idle surface therefore paints nothing at
//! all, and a burst of a hundred deltas paints at most thirty frames a second,
//! because everything that piled up while one frame was being painted folds into
//! the next one.
//!
//! The terminal is entered and left here, and a panic anywhere inside the loop
//! leaves it the way it was found: the hook restores first and then chains the
//! hook that was installed before it.

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use tokio::sync::mpsc;

use super::direct_run::DirectDeps;
use super::model::Model;
use super::msg::Msg;
use super::run_clock::{Clock, SystemClock};
use super::run_cmd::{CmdPort, SessionPort, perform};
use super::run_draw::Facts;
use super::run_front::Forward;
use super::run_input;
use super::run_panic::Restored;
use super::run_screen::{Screen, TtyScreen};
use super::update::update;
use crate::chat::{ChatSession, OpsContext};

/// The shortest gap between two frames: thirty a second, the ceiling the design
/// gives the render loop.
pub(crate) const FRAME: Duration = Duration::from_millis(33);

/// The surface's loop, and everything it owns.
pub(crate) struct Runtime<S: Screen, C: Clock, P: CmdPort> {
    model: Model,
    msgs: mpsc::UnboundedReceiver<Msg>,
    screen: S,
    clock: C,
    port: Arc<P>,
    facts: Facts,
    /// Whether the model has changed since the last frame was painted.
    dirty: bool,
    /// When the last frame was painted.
    painted: Option<Instant>,
    /// When the turn in flight started, for the status line's clock.
    working: Option<Instant>,
    /// Whether something on the surface moves; the ticker reads it.
    animating: Arc<AtomicBool>,
}

impl<S: Screen, C: Clock, P: CmdPort> Runtime<S, C, P> {
    /// A loop over one model, one queue, one terminal and one clock.
    pub(crate) fn new(
        model: Model,
        msgs: mpsc::UnboundedReceiver<Msg>,
        screen: S,
        clock: C,
        port: Arc<P>,
    ) -> Self {
        Self {
            model,
            msgs,
            screen,
            clock,
            port,
            facts: Facts::default(),
            dirty: false,
            painted: None,
            working: None,
            animating: Arc::new(AtomicBool::new(false)),
        }
    }

    /// The facts every frame is painted with: the launch's token column, glyph
    /// set, provider and workspace. A session that bound no workspace is asked
    /// the trust question here, once, before the loop reads a key (`02` §9.4).
    pub(crate) fn with_facts(mut self, facts: Facts) -> Self {
        if facts.untrusted {
            self.model.open_trust();
        }
        self.facts = facts;
        self
    }

    /// The flag the ticker reads: set while the surface has something that
    /// moves, so an idle surface is never woken.
    pub(crate) fn animating_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.animating)
    }

    /// Runs until the queue closes.
    pub(crate) async fn run(mut self) -> io::Result<()> {
        self.screen.enter()?;
        let restored = Restored::install(self.screen.restorer());
        let outcome = self.parade().await;
        drop(restored);
        let left = self.screen.restore();
        outcome.and(left)
    }

    /// One turn of the loop, with no waiting of its own: fold the message that
    /// is waiting, drain the rest of the burst into the same frame, and paint
    /// the frame that is due.
    pub(crate) async fn step(&mut self) -> io::Result<bool> {
        let Some(msg) = self.msgs.recv().await else {
            return Ok(false);
        };
        self.fold(msg)?;
        self.drain()?;
        self.paint_if_due()?;
        Ok(true)
    }

    /// Blocks on the queue, folding whatever arrives, and paints the frames the
    /// rate limit allows — including the one a burst held back, which is painted
    /// when its 33 ms are up even if nothing else arrives.
    async fn parade(&mut self) -> io::Result<()> {
        loop {
            match self.owed() {
                // A frame is owed and the limit is holding it: wait for the next
                // message or for the frame's turn, whichever comes first.
                Some(owed) => match tokio::time::timeout(owed, self.msgs.recv()).await {
                    Ok(Some(msg)) => self.fold(msg)?,
                    Ok(None) => return Ok(()),
                    Err(_) => {
                        self.paint()?;
                        continue;
                    }
                },
                None => match self.msgs.recv().await {
                    Some(msg) => self.fold(msg)?,
                    None => return Ok(()),
                },
            }
            self.drain()?;
            self.paint_if_due()?;
        }
    }

    /// Folds everything already queued into the frame that comes next: a burst
    /// of deltas is one paint, not a hundred.
    fn drain(&mut self) -> io::Result<()> {
        while let Ok(msg) = self.msgs.try_recv() {
            self.fold(msg)?;
        }
        Ok(())
    }

    /// One message: the pure update, its commands performed, and the surface
    /// marked as changed.
    fn fold(&mut self, msg: Msg) -> io::Result<()> {
        let busy = self.model.is_busy();
        let cmds = update(&mut self.model, msg);
        for cmd in cmds {
            perform(self.port.as_ref(), cmd);
        }
        self.dirty = true;
        match (busy, self.model.is_busy()) {
            (false, true) => self.working = Some(self.clock.now()),
            (true, false) => self.working = None,
            _ => {}
        }
        self.animating
            .store(run_input::animating(&self.model), Ordering::Relaxed);
        Ok(())
    }

    /// How long until a frame may be painted, when one is owed and the limit is
    /// holding it back.
    fn owed(&self) -> Option<Duration> {
        if !self.dirty {
            return None;
        }
        let elapsed = self.since_paint()?;
        (elapsed < FRAME).then(|| FRAME - elapsed)
    }

    /// How long ago the last frame was painted, if one was.
    fn since_paint(&self) -> Option<Duration> {
        let painted = self.painted?;
        Some(self.clock.now().saturating_duration_since(painted))
    }

    /// Paints the frame a change asks for, unless the last one is too recent.
    fn paint_if_due(&mut self) -> io::Result<()> {
        if !self.dirty {
            return Ok(());
        }
        if self.since_paint().is_some_and(|since| since < FRAME) {
            // Owed: the loop's own deadline paints it once its 33 ms are up.
            return Ok(());
        }
        self.paint()
    }

    /// Paints one frame and marks the surface unchanged.
    fn paint(&mut self) -> io::Result<()> {
        let facts = self.facts_now();
        self.screen.draw(&self.model, &facts)?;
        self.dirty = false;
        self.painted = Some(self.clock.now());
        Ok(())
    }

    /// The facts this frame is painted with: the launch's, plus how long the
    /// running turn has run.
    fn facts_now(&self) -> Facts {
        Facts {
            working: self
                .working
                .map(|started| self.clock.now().saturating_duration_since(started)),
            ..self.facts.clone()
        }
    }
}

/// The surface on the real terminal, with its producers: the input thread, the
/// sessions's front and the ticker.
///
/// The launch still opens the old TUI; T19 switches `saya` over to this call. It
/// is exported with the boundary's other names, and the attribute says why
/// nothing references it yet.
#[allow(dead_code)]
pub(crate) async fn run(
    model: Model,
    session: Arc<ChatSession>,
    ops: OpsContext,
    direct: Arc<DirectDeps>,
    facts: Facts,
) -> io::Result<()> {
    let (msgs, queue) = mpsc::unbounded_channel();
    let front = Arc::new(Forward::new(msgs.clone()));
    let port = Arc::new(SessionPort::new(session, front, ops, direct));
    let runtime = Runtime::new(model, queue, TtyScreen::new(), SystemClock, port).with_facts(facts);
    run_input::spawn_input(msgs.clone());
    run_input::spawn_ticker(runtime.animating_flag(), msgs, super::view::TICK);
    runtime.run().await
}
